//! Sobe o kernel na QEMU: interativo (`run`) ou como teste de fumaça (`smoke`).

use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::arch::Arch;
use crate::cargo::{self, Profile};
use crate::{image, leak, panic_smoke, png, workspace};

/// Igual ao `BOOT_OK_MARKER` do kernel; os dois mudam juntos.
pub const BOOT_OK: &str = "[rustos] boot ok";
const PANIC: &str = "[rustos] panic";
/// Linhas que o kernel imprime quando um autoteste ou o boot falha.
const FATAL: [&str; 3] = ["[rustos] fault:", "[rustos] interrupts:", "FAILED"];
/// A linha em que o kernel diz que a imagem foi sorteada, sem dizer onde está (SDD 13).
const KASLR_ON: &str = "[rustos] kaslr: on";
/// Com `debug-addrs` o kernel imprime a base; com o KASLR ligado ela nunca é a do linker script.
const LINK_BASE: &str = "kernel base: 0xffffffff80000000 ";
/// Linhas que todo boot precisa ter (guidelines 09): as proteções de hardware, o
/// W^X conferido nas tabelas de página, a memória que o bootloader devolveu e a
/// preempção com o estado de SIMD por tarefa.
const REQUIRED: [(&str, &str); 7] = [
    ("[rustos] protections:", "a linha de proteções de hardware"),
    (
        "[rustos] mitigations:",
        "a linha de mitigações de CPU (vulnerabilidades e o que o kernel liga)",
    ),
    (
        "[rustos] w^x: ok",
        "a conferência de W^X das tabelas de página",
    ),
    ("[rustos] reclaimed:", "a memória devolvida pelo bootloader"),
    (
        "[rustos] task self-test: ok",
        "o autoteste de preempção e de estado de SIMD por tarefa",
    ),
    (
        "[rustos] smp self-test: ok",
        "o autoteste de migração entre núcleos",
    ),
    (
        "[rustos] wake self-test: ok",
        "o autoteste de acordar uma tarefa de fatia curta na mesma CPU",
    ),
];
/// A saída dos programas de usuário pelo `debug_write` (só o modo de teste os lança): o kernel a
/// prefixa com `[user] `. As do `init` provam o canal de boot e o console por handle; a do `handles`,
/// os casos hostis das chamadas de handle, canal e espera.
const USER_OUTPUT: [(&str, &str); 3] = [
    (
        "[user] init: boot channel, console and handles ok",
        "a prova do `init` de que o canal de boot e o console por handle funcionam",
    ),
    ("[user] init: ok", "a saída do `init` (prefixo `[user] `)"),
    (
        "[user] handles: ok (",
        "os casos hostis de handle, canal e espera",
    ),
];
const SMOKE_TIMEOUT: Duration = Duration::from_secs(120);

/// Como o `smoke` sobe o kernel: com os autotestes e os benchmarks, ou só o essencial (SDD 06).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootMode {
    /// `rustos.selftest=1` na linha de comando: o que o `make smoke` confere.
    SelfTest,
    /// Sem linha de comando: o boot que o usuário vê, o do `make smoke-fast`.
    Normal,
    /// `rustos.selftest=1` numa imagem sem a feature `selftest`: o kernel recusa em uma linha e
    /// sobe normal, e o `bench` do shell diz que não está no build (SDD 06).
    Refused,
}

impl BootMode {
    const fn cmdline(self) -> Option<&'static str> {
        match self {
            Self::SelfTest | Self::Refused => Some("rustos.selftest=1"),
            Self::Normal => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::SelfTest => "smoke",
            Self::Normal => "smoke-fast",
            Self::Refused => "smoke-fast (selftest recusado)",
        }
    }
}

/// O que o kernel enxuto imprime ao recusar o modo de teste, e o que o `bench` do shell responde.
const SELFTEST_REFUSED: &str = "[rustos] selftest: not in this build";
const BENCH_REFUSED: &str = "bench: not in this build";

/// Um comando que o smoke digita no shell e o que a resposta dele (até o prompt voltar) tem de
/// conter, em texto puro, sem as sequências de escape.
struct Step {
    command: &'static str,
    expect: &'static [&'static str],
}

/// A sessão do M1 (SDD 17): listar o initramfs, rodar um programa como processo, ver o processo no
/// `ps`, e um programa que morre contido sem derrubar o sistema. Vale no boot normal e no de teste.
const SESSION: [Step; 9] = [
    Step {
        command: "ls",
        expect: &["hello", "crash", "sysinfo", "count", "spin", "init"],
    },
    Step {
        command: "run hello",
        expect: &[
            "[user] hello from a user process (",
            "exited (code 0,",
            "exit code 0 (pid ",
        ],
    },
    Step {
        command: "run sysinfo",
        expect: &["[user] sysinfo: up ", "exit code 0 (pid "],
    },
    Step {
        command: "run crash",
        expect: &[
            "[user] crash: going down the stack",
            "[rustos] user: fault contained (pid ",
            "killed: page fault",
        ],
    },
    Step {
        command: "run nothere",
        expect: &["run: nothere: no such program"],
    },
    Step {
        command: "run count 2",
        expect: &[
            "count: 1/2",
            "count: 2/2",
            "count: done",
            "exit code 0 (pid ",
        ],
    },
    Step {
        command: "run count x",
        expect: &["count: usage:", "exit code 2 (pid "],
    },
    Step {
        command: "run count &",
        expect: &["started pid "],
    },
    Step {
        command: "ps",
        expect: &["pid", "shell"],
    },
];
/// A última linha de `count`, que prova que o programa em segundo plano terminou sozinho.
const COUNT_DONE: &str = "[user] count: done";
/// O comando que lança o `count` em segundo plano: a espera pela última linha começa depois dele, porque
/// o `run count 2` da sessão já imprimiu a sua.
const COUNT_BACKGROUND: &str = "run count &";
/// A linha com que `spin` começa a girar: o `run spin` só é interrompido depois dela.
const SPIN_LINE: &str = "[user] spin: spinning";
/// `count` conta cinco vezes meio segundo; sob TCG sobra folga.
const COUNT_TIMEOUT: Duration = Duration::from_secs(20);

/// O que só o modo de teste imprime: se uma destas faltar, a linha de comando se perdeu e o
/// boot que passou foi o normal.
const SELFTEST_LINES: [(&str, &str); 23] = [
    (
        "[rustos] cpu features: ",
        "os recursos de CPU das mitigações",
    ),
    (
        "[rustos] user: entered ring3/el0",
        "a entrada do primeiro programa em anel 3 ou EL0",
    ),
    (
        "[rustos] user: init loaded (",
        "o `init` carregado do initramfs",
    ),
    (
        "[rustos] user: pid 1 exited (code 0,",
        "o `init` (pid 1) saindo com 0",
    ),
    ("[rustos] frame self-test: ok", "o autoteste de frame"),
    ("[rustos] heap self-test: ok", "o autoteste do heap"),
    ("[rustos] exception self-test: ok", "o autoteste de exceção"),
    ("[rustos] timer: ", "o autoteste do timer"),
    ("[rustos] idle: ", "a medida da ociosa"),
    ("[rustos] key residue: ", "o autoteste de resíduo de chave"),
    ("[rustos] zero bench: ", "a medida de zerar"),
    ("[rustos] random bench: ", "a medida do gerador"),
    ("[rustos] switch: ", "a medida de troca de contexto"),
    ("[rustos] scale: ", "a medida de escala"),
    ("[rustos] alloc storm: ", "a medida de contenção da memória"),
    ("[rustos] stacks: ", "a medida e a prova de pilha"),
    (
        "[rustos] user: fault contained (pid ",
        "a falha de usuário que mata só o processo",
    ),
    (
        "[rustos] user self-test: ok",
        "o autoteste dos casos de usuário",
    ),
    (
        "[rustos] initramfs: 11 files, ",
        "a leitura do initramfs (o `init`, as cinco demonstrações e os cinco de teste)",
    ),
    (
        "[user] lazy: ok (",
        "o `lazy`: pilha e `.bss` sob demanda, com a página zero e a cópia dela",
    ),
    (
        "[rustos] user: demand paging: ",
        "as faltas de página resolvidas pelo kernel (página zero, frame novo, cópia)",
    ),
    (
        "[rustos] tlb shootdown: ok (cpu ",
        "o teste de invalidação do TLB entre CPUs",
    ),
    (
        "[rustos] tlb shootdown ring: ok (",
        "o estresse de invalidação do TLB em anel com todas as CPUs",
    ),
];
/// O que o boot normal confere, além do que o `REQUIRED` tem de comum aos dois modos.
const NORMAL_LINES: [(&str, &str); 2] = [
    ("[rustos] frame self-test: ok", "o autoteste de frame"),
    ("[rustos] exception self-test: ok", "o autoteste de exceção"),
];
/// O boot normal de uma imagem enxuta que recebeu `rustos.selftest=1`.
const REFUSED_LINES: [(&str, &str); 4] = [
    NORMAL_LINES[0],
    NORMAL_LINES[1],
    (SELFTEST_REFUSED, "a recusa do modo de teste"),
    (BENCH_REFUSED, "a recusa do `bench` no shell"),
];
/// A linha de tempo do boot, nos dois modos (SDD 04).
const BOOT_TIME: &str = "[rustos] boot time: kernel ";
/// Autotestes de tarefa que o `REQUIRED` pode ter e que só o modo de teste roda.
const TASK_SELFTESTS: [&str; 2] = ["[rustos] smp self-test:", "[rustos] wake self-test:"];

/// Qualquer linha do initramfs: no boot normal nem a de sucesso (que é do modo de teste) nem o
/// aviso `not mapped` pode aparecer.
const INITRAMFS_ANY: &str = "[rustos] initramfs:";

/// O que o boot normal não pode imprimir: tudo o que é do modo de teste e não do normal, mais
/// os autotestes de tarefa. Uma linha destas é trabalho de teste que voltou ao caminho do prompt.
fn normal_forbidden() -> Vec<&'static str> {
    let own = |line: &str| NORMAL_LINES.iter().any(|(kept, _)| *kept == line);
    SELFTEST_LINES
        .iter()
        .map(|(line, _)| *line)
        .filter(|line| !own(line))
        .chain(["[rustos] task self-test:", INITRAMFS_ANY])
        .chain(TASK_SELFTESTS)
        .collect()
}

/// Teto do `boot time:` no modo normal, em ms. Só uma aceleração **forçada** (`RUSTOS_ACCEL`) conta
/// como confirmada: a lista padrão tem `kvm`/`hvf` mas a QEMU cai no TCG sem avisar (o runner do CI
/// tem `/dev/kvm` sem permissão). Sem confirmação o teto é folgado e as linhas proibidas pegam a
/// regressão de verdade.
fn normal_ceiling_ms(forced: Option<&str>) -> u64 {
    match forced {
        Some("hvf" | "kvm") => 50,
        _ => 500,
    }
}

/// Teto do `firmware` do `boot timeline:` no aarch64, em ms, com qualquer aceleração (HVF 0,1 s,
/// TCG 0,5 s). A espera de 5 s do EDK2 (SDD 05) estoura.
const FIRMWARE_CEILING_MS: u64 = 4000;

/// Os microssegundos de `boot timeline: firmware N us`.
fn firmware_boot_us(log: &str) -> Option<u64> {
    const MARK: &str = "[rustos] boot timeline: firmware ";
    let rest = &kernel_line(log, MARK)?[MARK.len()..];
    rest.split_ascii_whitespace().next()?.parse().ok()
}

/// A linha em que o kernel diz quantos núcleos subiram; o limite do kernel (8) corta o pedido.
fn cpus_line(requested: usize) -> String {
    format!("[rustos] cpus: {} online", requested.min(MAX_KERNEL_CPUS))
}

/// Igual a `MAX_CPUS` de `core/hal`; o xtask não depende de crate do workspace.
const MAX_KERNEL_CPUS: usize = 8;

fn command(root: &Path, arch: Arch, profile: Profile) -> Result<Command, String> {
    command_with(root, arch, profile, &cargo::features(profile), None)
}

pub fn command_with(
    root: &Path,
    arch: Arch,
    profile: Profile,
    features: &[String],
    cmdline: Option<&str>,
) -> Result<Command, String> {
    let esp = image::assemble_with(root, arch, profile, features, cmdline)?;
    let firmware = arch.firmware()?;
    let mut qemu = Command::new(arch.qemu());
    qemu.args(arch.qemu_machine_args(&firmware.to_string_lossy(), &esp.to_string_lossy()));
    qemu.arg("-no-reboot");
    Ok(qemu)
}

pub fn run(root: &Path, arch: Arch, profile: Profile) -> Result<(), String> {
    let mut qemu = command(root, arch, profile)?;
    println!(
        "xtask: {} na QEMU; a tela abre numa janela, o shell lê daqui; Ctrl-A X encerra",
        arch.name()
    );
    workspace::run(qemu.args(["-serial", "mon:stdio"]))
}

/// O `smoke-fast`: o boot normal e, na imagem sem a feature `selftest`, o que ela promete no lugar
/// do modo de teste (a recusa do `rustos.selftest=1` e a do `bench`). Assim a imagem enxuta, que
/// o `make smoke` não cobre, não apodrece.
pub fn smoke_fast(root: &Path, arch: Arch, profile: Profile) -> Result<(), String> {
    smoke(root, arch, profile, BootMode::Normal)?;
    if cargo::has_feature(&cargo::features(profile), cargo::SELFTEST) {
        return Ok(());
    }
    smoke(root, arch, profile, BootMode::Refused)
}

pub fn smoke(root: &Path, arch: Arch, profile: Profile, mode: BootMode) -> Result<(), String> {
    let features = match mode {
        BootMode::SelfTest => cargo::features_with_selftest(profile),
        BootMode::Normal | BootMode::Refused => cargo::features(profile),
    };
    let mut qemu = command_with(root, arch, profile, &features, mode.cmdline())?;
    qemu.args(["-display", "none", "-serial", "stdio", "-monitor", "none"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let started = Instant::now();
    let mut child = qemu
        .spawn()
        .map_err(|error| format!("não consegui rodar {}: {error}", arch.qemu()))?;
    let receiver = stream_stdout(&mut child)?;
    let (outcome, mut log) = wait_for_boot(&receiver, started);
    let boot_log = log.clone();
    let mut session = Ok(());
    if matches!(outcome, Outcome::Booted(_)) {
        if mode == BootMode::Refused {
            type_lean_commands(&mut child, &receiver, &mut log);
        } else {
            session = type_session(&mut child, &receiver, &mut log);
        }
    }
    child.kill().ok();
    child.wait().ok();

    print_kernel_lines(&log);
    // O modo normal não pode ter rodado nada do modo de teste no boot (`boot_log`); a sessão
    // digitada tem as próprias exigências e a lista de proibidas dela vale sobre o log inteiro.
    let judged = if mode == BootMode::Refused {
        &log
    } else {
        &boot_log
    };
    report(arch, mode, outcome, judged)?;
    session
        .and_then(|()| check_session_forbidden(mode, &log))
        .and_then(|()| check_no_kernel_address(&log[boot_log.len()..]))
        .map_err(|reason| failure(arch, &reason, &log))
}

/// As linhas que a sessão do shell produz de propósito no boot normal: lançar um processo imprime
/// a entrada em usuário, o fim e a falha contida.
const SESSION_OWN: [&str; 3] = [
    "[rustos] user: entered ring3/el0",
    "[rustos] user: pid 1 exited (code 0,",
    "[rustos] user: fault contained (pid ",
];

/// O que o boot normal não imprime nem depois da sessão: `normal_forbidden` menos o que a sessão
/// produz (`SESSION_OWN`). `init loaded`, `user self-test` e o resto do modo de teste seguem proibidos.
fn session_forbidden() -> Vec<&'static str> {
    normal_forbidden()
        .into_iter()
        .filter(|line| !SESSION_OWN.contains(line))
        .collect()
}

/// No boot normal, o log inteiro (boot e sessão) não tem linha do modo de teste.
fn check_session_forbidden(mode: BootMode, log: &str) -> Result<(), String> {
    if mode != BootMode::Normal {
        return Ok(());
    }
    match session_forbidden().iter().find(|line| has_mark(log, line)) {
        Some(line) => Err(format!(
            "o boot normal, com a sessão do shell, imprimiu `{line}`, que é do modo de teste"
        )),
        None => Ok(()),
    }
}

/// O endereço do kernel que a sessão digitada deixou no log, se deixou (SDD 09).
fn check_no_kernel_address(session: &str) -> Result<(), String> {
    if cargo::has_feature(&cargo::listed_features(), "debug-addrs") {
        return Ok(());
    }
    match leak::find_kernel_address(session) {
        Some(address) => Err(format!(
            "endereço do kernel na sessão do shell: `{address}`"
        )),
        None => Ok(()),
    }
}

/// Quantos bytes antes do que o `type_command` digita cabem o prompt anterior, com o `% ` que o eco
/// segue: o prompt pode chegar em pedaços, e o `% ` ficar antes do trecho lido.
const PROMPT_WINDOW: usize = 48;

/// Digita `command` e lê para o `log` até o prompt voltar. Devolve a resposta em texto puro.
fn type_command(
    child: &mut std::process::Child,
    receiver: &mpsc::Receiver<String>,
    log: &mut String,
    command: &str,
) -> Result<String, String> {
    let from = log.len();
    let stdin = child.stdin.as_mut().ok_or("QEMU sem stdin")?;
    stdin
        .write_all(format!("{command}\r").as_bytes())
        .and_then(|()| stdin.flush())
        .map_err(|error| format!("digitar `{command}` na serial: {error}"))?;
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let window = log.floor_char_boundary(from.saturating_sub(PROMPT_WINDOW));
    while !answered(&log[window..], command) {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        match receiver.recv_timeout(left) {
            Ok(chunk) => log.push_str(&chunk),
            Err(_) => break,
        }
    }
    if !answered(&log[window..], command) {
        return Err(format!(
            "o shell não devolveu o prompt depois de `{command}`"
        ));
    }
    Ok(strip_escapes(&log[from..]))
}

/// Digita a `SESSION` no shell, uma linha por vez, e confere cada resposta. No fim espera o
/// `count` em segundo plano terminar sozinho. Escreve na tela o que cada comando respondeu.
fn type_session(
    child: &mut std::process::Child,
    receiver: &mpsc::Receiver<String>,
    log: &mut String,
) -> Result<(), String> {
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    read_to_prompt(receiver, log, 0, deadline);
    if !ends_with_prompt(log) {
        return Err("o shell não mostrou o prompt depois do boot".to_owned());
    }
    let mut background_from = 0;
    for step in &SESSION {
        if step.command == COUNT_BACKGROUND {
            background_from = log.len();
        }
        let answer = type_command(child, receiver, log, step.command)?;
        if let Some(missing) = step.expect.iter().find(|text| !answer.contains(**text)) {
            return Err(format!(
                "`{}` respondeu sem `{missing}`:\n{answer}",
                step.command
            ));
        }
        if step.command == "ps" {
            if !ps_shows_a_process(&answer) {
                return Err(format!(
                    "o `ps` não mostrou o processo `count` com o pid:\n{answer}"
                ));
            }
            if !ps_table_is_whole(&answer) {
                return Err(format!(
                    "a saída de um programa caiu no meio da tabela do `ps`:\n{answer}"
                ));
            }
        }
        println!("xtask: `{}` ok", step.command);
    }
    type_interrupt_and_kill(child, receiver, log)?;
    println!("xtask: Ctrl-C no `run` e `kill <pid>` ok");
    let deadline = Instant::now() + COUNT_TIMEOUT;
    while !has_user_line(&log[background_from..], COUNT_DONE) {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            return Err(format!(
                "o `count` em segundo plano não terminou (`{COUNT_DONE}`)"
            ));
        };
        match receiver.recv_timeout(left) {
            Ok(chunk) => log.push_str(&chunk),
            Err(_) => return Err("a serial fechou antes de o `count` terminar".to_owned()),
        }
    }
    check_no_prompt_glue(log)?;
    check_no_heap_leak(child, receiver, log)
}

/// Digita `run spin`, espera o programa girar, manda Ctrl-C e confere que o shell voltou ao prompt. Depois
/// lança `spin` em segundo plano, lê o pid e o mata com `kill`, e espera o kernel dizer que morreu.
fn type_interrupt_and_kill(
    child: &mut std::process::Child,
    receiver: &mpsc::Receiver<String>,
    log: &mut String,
) -> Result<(), String> {
    let from = log.len();
    send(child, "run spin\r")?;
    wait_until(receiver, log, from, |text| has_user_line(text, SPIN_LINE))
        .map_err(|()| format!("`run spin` não imprimiu `{SPIN_LINE}`"))?;

    let from = log.len();
    send(child, "\x03")?;
    read_to_prompt(receiver, log, from, Instant::now() + COMMAND_TIMEOUT);
    let answer = strip_escapes(&log[from..]);
    if !answer.contains("interrupted (pid ") || !ends_with_prompt(&log[from..]) {
        return Err(format!(
            "Ctrl-C não interrompeu o `run spin` e não devolveu o prompt:\n{answer}"
        ));
    }

    let answer = type_command(child, receiver, log, "run spin &")?;
    let pid =
        started_pid(&answer).ok_or_else(|| format!("`run spin &` não disse o pid:\n{answer}"))?;
    let from = log.len();
    let answer = type_command(child, receiver, log, &format!("kill {pid}"))?;
    if !answer.contains(&format!("killed pid {pid}")) {
        return Err(format!("`kill {pid}` não confirmou a morte:\n{answer}"));
    }
    let death = format!("[rustos] user: pid {pid} killed (");
    wait_until(receiver, log, from, |text| text.contains(&death))
        .map_err(|()| format!("o kernel não registrou a morte do pid {pid} (`{death}`)"))
}

/// Manda `bytes` pela serial da QEMU, sem Enter.
fn send(child: &mut std::process::Child, bytes: &str) -> Result<(), String> {
    let stdin = child.stdin.as_mut().ok_or("QEMU sem stdin")?;
    stdin
        .write_all(bytes.as_bytes())
        .and_then(|()| stdin.flush())
        .map_err(|error| format!("mandar `{bytes:?}` pela serial: {error}"))
}

/// O pid que `run <programa> &` respondeu (`started pid N`).
fn started_pid(answer: &str) -> Option<u32> {
    let rest = answer.split("started pid ").nth(1)?;
    rest.split_whitespace().next()?.parse().ok()
}

/// Lê para o `log` até `done` dizer que o texto desde `from` basta, ou o prazo acabar.
fn wait_until(
    receiver: &mpsc::Receiver<String>,
    log: &mut String,
    from: usize,
    done: impl Fn(&str) -> bool,
) -> Result<(), ()> {
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    while !done(&log[from..]) {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            return Err(());
        };
        match receiver.recv_timeout(left) {
            Ok(chunk) => log.push_str(&chunk),
            Err(_) => return Err(()),
        }
    }
    Ok(())
}

/// Quantos `run hello` entre os dois `mem` do teste de vazamento.
const LEAK_RUNS: usize = 20;

/// Cada processo que nasce e morre tem de devolver todo o heap do kernel: aquece com um `run hello`,
/// lê o heap em uso no `mem`, roda `LEAK_RUNS` vezes e lê de novo. Um vazamento de centenas de bytes
/// por processo sobe a leitura em alguns KiB, acima da resolução de 0,1 KiB do `mem`.
fn check_no_heap_leak(
    child: &mut std::process::Child,
    receiver: &mpsc::Receiver<String>,
    log: &mut String,
) -> Result<(), String> {
    type_command(child, receiver, log, "run hello")?;
    let read = |child: &mut std::process::Child, log: &mut String| {
        let answer = type_command(child, receiver, log, "mem")?;
        heap_used(&answer).ok_or(format!("o `mem` não mostrou o heap em uso:\n{answer}"))
    };
    let before = read(child, log)?;
    for _ in 0..LEAK_RUNS {
        let answer = type_command(child, receiver, log, "run hello")?;
        if !answer.contains("exit code 0 (pid ") {
            return Err(format!("`run hello` não saiu com 0:\n{answer}"));
        }
    }
    let after = read(child, log)?;
    if before != after {
        return Err(format!(
            "o heap do kernel passou de {before} para {after} em {LEAK_RUNS} `run hello`: \
             o processo vaza memória ao morrer"
        ));
    }
    println!("xtask: heap em {after} antes e depois de {LEAK_RUNS} `run hello`");
    Ok(())
}

/// O heap em uso que o `mem` mostra (`heap  18.1 KiB  (11 pages, …)`), como texto: `18.1 KiB`.
fn heap_used(answer: &str) -> Option<String> {
    let line = answer
        .lines()
        .find(|line| line.split_whitespace().next() == Some("heap"))?;
    let mut words = line.split_whitespace().skip(1);
    let (number, unit) = (words.next()?, words.next()?);
    number.parse::<f64>().ok()?;
    Some(format!("{number} {unit}"))
}

/// Uma linha do `ps` do processo `count` (o que `run count &` lançou) com o número do processo na
/// coluna `pid` (a terceira).
fn ps_shows_a_process(answer: &str) -> bool {
    answer.lines().any(|line| {
        let mut words = line.split_whitespace().skip(1);
        words.next() == Some("count") && words.next().is_some_and(|pid| pid.parse::<u32>().is_ok())
    })
}

/// A tabela de tarefas do `ps`, do cabeçalho até o cabeçalho da tabela de CPUs, não tem linha de
/// usuário no meio: a saída de um programa em segundo plano espera o `ps` acabar.
fn ps_table_is_whole(answer: &str) -> bool {
    let mut lines = answer.lines().skip_while(|line| !line.contains("switches"));
    lines.by_ref().next().is_some()
        && lines
            .take_while(|line| !line.contains("ipi"))
            .all(|line| !line.contains("[user]"))
}

/// A resposta de `command` terminou: o eco dele depois de um prompt, seguido do Enter, e depois
/// outro prompt. Em texto puro, porque um prompt que o shell redesenha no meio da digitação
/// (saída de segundo plano) parte o eco em pedaços que o terminal junta.
fn answered(log: &str, command: &str) -> bool {
    let plain = strip_escapes(log);
    plain
        .find(&format!("% {command}\n"))
        .is_some_and(|at| plain[at..].contains("rustos ~ % "))
}

/// Nenhuma linha começa com o prompt colado em texto de usuário: o shell apaga a linha do prompt
/// (`\r`) antes de a saída de segundo plano entrar. A tela de um terminal mostra o que vem depois
/// do último `\r` da linha.
fn check_no_prompt_glue(log: &str) -> Result<(), String> {
    for line in log.split('\n').map(|line| line.trim_end_matches('\r')) {
        let visible = strip_escapes(line.rsplit('\r').next().unwrap_or(line));
        if visible.contains("rustos ~ % [user]") {
            return Err(format!("saída de usuário colada no prompt: `{visible}`"));
        }
    }
    Ok(())
}

/// Lê para o `log` até o prompt do shell voltar depois de `from`, ou o prazo acabar.
fn read_to_prompt(
    receiver: &mpsc::Receiver<String>,
    log: &mut String,
    from: usize,
    deadline: Instant,
) {
    while !ends_with_prompt(&log[from..]) {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            return;
        };
        match receiver.recv_timeout(left) {
            Ok(chunk) => log.push_str(&chunk),
            Err(_) => return,
        }
    }
}

/// Digita `bench` e depois `ps` no shell da imagem enxuta, cada um quando o prompt volta, e lê as
/// respostas para o `log`: a recusa do `bench` e a coluna de pilha que o gate confere.
fn type_lean_commands(
    child: &mut std::process::Child,
    receiver: &mpsc::Receiver<String>,
    log: &mut String,
) {
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    read_to_prompt(receiver, log, 0, deadline);
    for command in ["bench", "ps"] {
        let from = log.len();
        if let Some(stdin) = child.stdin.as_mut() {
            stdin
                .write_all(format!("{command}\r").as_bytes())
                .and_then(|()| stdin.flush())
                .ok();
        }
        read_to_prompt(receiver, log, from, deadline);
    }
}

/// Sobe sem janela, espera o boot, digita cada comando na serial e grava a tela
/// pelo monitor da QEMU (PPM), convertida para PNG. Devolve o caminho do PNG.
pub fn screenshot(
    root: &Path,
    arch: Arch,
    profile: Profile,
    commands: &[String],
) -> Result<PathBuf, String> {
    let dir = root.join("target").join("screenshots");
    fs::create_dir_all(&dir).map_err(|error| format!("criar {}: {error}", dir.display()))?;
    let ppm = dir.join(format!("{}.ppm", arch.name()));
    let socket = dir.join(format!("{}.monitor", arch.name()));
    fs::remove_file(&ppm).ok();
    fs::remove_file(&socket).ok();

    let mut qemu = command(root, arch, profile)?;
    let monitor = format!("unix:{},server=on,wait=off", socket.display());
    qemu.args(["-display", "none", "-serial", "stdio", "-monitor", &monitor])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let started = Instant::now();
    let mut child = qemu
        .spawn()
        .map_err(|error| format!("não consegui rodar {}: {error}", arch.qemu()))?;
    let receiver = stream_stdout(&mut child)?;
    let (outcome, log) = wait_for_boot(&receiver, started);
    let result = match outcome {
        Outcome::Booted(_) => capture(&mut child, &receiver, &socket, &ppm, commands),
        other => {
            report(arch, BootMode::Normal, other, &log).and(Err("sem boot, sem captura".to_owned()))
        }
    };
    child.kill().ok();
    child.wait().ok();
    fs::remove_file(&socket).ok();
    result?;
    let bytes = fs::read(&ppm).map_err(|error| format!("ler {}: {error}", ppm.display()))?;
    let (width, height, rgb) = png::decode_ppm(&bytes)?;
    let png_path = ppm.with_extension("png");
    fs::write(&png_path, png::encode_rgb(width, height, rgb)?)
        .map_err(|error| format!("gravar {}: {error}", png_path.display()))?;
    fs::remove_file(&ppm).ok();
    Ok(png_path)
}

/// Tempo para o kernel desenhar depois de cada entrada; TCG é lento.
const SETTLE: Duration = Duration::from_millis(1500);

/// O fim do prompt do shell (`rustos ~ % `): quando ele volta, o comando terminou.
pub const PROMPT_TAIL: &str = "\x1b[34m~\x1b[0m % ";
/// Tempo máximo que um comando do shell leva para devolver o prompt.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Espera o prompt voltar depois de um comando digitado; o `bench` leva mais que `SETTLE`.
/// Quem chama descarta antes o que já chegou: o prompt anterior não conta.
fn wait_for_prompt(receiver: &mpsc::Receiver<String>) -> String {
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let mut seen = String::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match receiver.recv_timeout(left) {
            Ok(chunk) => seen.push_str(&chunk),
            Err(_) => break,
        }
        if ends_with_prompt(&seen) {
            break;
        }
    }
    seen
}

/// O texto sem as sequências de escape do terminal (`ESC [ … letra`), para imprimir no log.
fn strip_escapes(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\x1b' {
            plain.push(ch);
        } else if chars.next() == Some('[') {
            chars.by_ref().find(char::is_ascii_alphabetic);
        }
    }
    plain.replace('\r', "")
}

pub fn ends_with_prompt(log: &str) -> bool {
    log.contains(PROMPT_TAIL)
}

fn capture(
    child: &mut std::process::Child,
    receiver: &mpsc::Receiver<String>,
    socket: &Path,
    ppm: &Path,
    commands: &[String],
) -> Result<(), String> {
    thread::sleep(SETTLE);
    let mut monitor =
        UnixStream::connect(socket).map_err(|error| format!("monitor da QEMU: {error}"))?;
    let stdin = child.stdin.as_mut().ok_or("QEMU sem stdin")?;
    let typed = commands
        .iter()
        .flat_map(|arg| arg.split(';'))
        .map(str::trim)
        .filter(|command| !command.is_empty());
    for command in typed {
        if let Some(keys) = command.strip_prefix(KEYS_PREFIX) {
            press_keys(&mut monitor, keys)?;
        } else {
            while receiver.try_recv().is_ok() {}
            stdin
                .write_all(format!("{command}\r").as_bytes())
                .and_then(|()| stdin.flush())
                .map_err(|error| format!("digitar na serial: {error}"))?;
            let output = wait_for_prompt(receiver);
            println!("xtask: `{command}` respondeu:\n{}", strip_escapes(&output));
        }
        thread::sleep(SETTLE);
    }
    writeln!(monitor, "screendump {}", ppm.display())
        .map_err(|error| format!("screendump: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if fs::metadata(ppm).is_ok_and(|meta| meta.len() > 0) {
            thread::sleep(Duration::from_millis(300));
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    Err(format!("a QEMU não gravou {}", ppm.display()))
}

/// Comando que começa assim vai pelo teclado da máquina (`sendkey` do monitor),
/// em qcodes da QEMU separados por espaço: `keys: e c h o spc semicolon ret`.
const KEYS_PREFIX: &str = "keys:";
const KEY_GAP: Duration = Duration::from_millis(60);

fn press_keys(monitor: &mut UnixStream, keys: &str) -> Result<(), String> {
    for key in keys.split_whitespace() {
        writeln!(monitor, "sendkey {key}").map_err(|error| format!("sendkey {key}: {error}"))?;
        thread::sleep(KEY_GAP);
    }
    Ok(())
}

pub fn stream_stdout(child: &mut std::process::Child) -> Result<mpsc::Receiver<String>, String> {
    let stdout = child.stdout.take().ok_or("QEMU sem stdout")?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || stream_chunks(stdout, &sender));
    Ok(receiver)
}

/// O que um terminal mostra no começo da linha: depois do último `\r` (o shell apaga a linha do
/// prompt antes de a saída de segundo plano entrar) e sem as sequências de apagar que a abrem.
fn shown(line: &str) -> &str {
    let line = line.trim_end_matches('\r');
    let mut text = line.rsplit('\r').next().unwrap_or(line);
    while let Some(rest) = text.strip_prefix("\x1b[") {
        let end = rest.find(|ch: char| ch.is_ascii_alphabetic());
        match end {
            Some(at) => text = &rest[at + 1..],
            None => break,
        }
    }
    text
}

/// As linhas do kernel: a linha como o terminal a mostra **começa** com `[rustos]`. Texto de
/// usuário (`debug_write`, sempre prefixado por `[user]`) e lixo de firmware no meio da linha não
/// contam.
fn kernel_lines(log: &str) -> impl Iterator<Item = &str> {
    log.split('\n')
        .map(shown)
        .filter(|line| line.starts_with("[rustos]"))
}

/// Alguma linha do kernel começa com `mark` (um prefixo `[rustos] …`); qualquer outro texto (a
/// resposta do shell, a base do kernel) se procura no log inteiro.
fn has_mark(log: &str, mark: &str) -> bool {
    if mark.starts_with("[rustos]") {
        kernel_lines(log).any(|line| line.starts_with(mark))
    } else {
        log.contains(mark)
    }
}

/// Uma linha fatal do kernel: o prefixo de `FATAL` no começo, ou `FAILED` dentro de uma linha do
/// kernel (o texto de usuário, prefixado por `[user]`, não conta).
fn has_fatal(log: &str) -> bool {
    FATAL.iter().any(|mark| {
        if mark.starts_with("[rustos]") {
            has_mark(log, mark)
        } else {
            kernel_lines(log).any(|line| line.contains(mark))
        }
    })
}

/// Alguma linha de usuário (`debug_write`: prefixo `[user] `) começa com `mark`.
fn has_user_line(log: &str, mark: &str) -> bool {
    log.split('\n')
        .map(shown)
        .any(|line| line.starts_with(mark))
}

/// A primeira linha do kernel que começa com `mark`.
fn kernel_line<'a>(log: &'a str, mark: &str) -> Option<&'a str> {
    kernel_lines(log).find(|line| line.starts_with(mark))
}

fn wait_for_boot(receiver: &mpsc::Receiver<String>, started: Instant) -> (Outcome, String) {
    let mut log = String::new();
    let outcome = loop {
        let remaining = SMOKE_TIMEOUT.saturating_sub(started.elapsed());
        match receiver.recv_timeout(remaining) {
            Ok(chunk) => log.push_str(&chunk),
            Err(mpsc::RecvTimeoutError::Timeout) => break Outcome::TimedOut,
            Err(mpsc::RecvTimeoutError::Disconnected) => break Outcome::Exited,
        }
        if has_mark(&log, BOOT_OK) {
            break Outcome::Booted(started.elapsed());
        }
        if has_mark(&log, PANIC) || has_fatal(&log) {
            drain(receiver, &mut log, FATAL_DRAIN);
            break Outcome::Panicked;
        }
    };
    (outcome, log)
}

/// Quanto o xtask ainda lê depois de casar uma linha fatal: o resto dela (a mensagem inteira) chega
/// depois do prefixo que casou.
const FATAL_DRAIN: Duration = Duration::from_millis(200);

fn drain(receiver: &mpsc::Receiver<String>, log: &mut String, window: Duration) {
    let deadline = Instant::now() + window;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match receiver.recv_timeout(left) {
            Ok(chunk) => log.push_str(&chunk),
            Err(_) => return,
        }
    }
}

/// As linhas que o modo exige, na ordem: as comuns do `REQUIRED` (sem os autotestes de tarefa,
/// que só o modo de teste roda), as do modo e a de tempo.
fn mode_lines(mode: BootMode) -> Vec<(&'static str, &'static str)> {
    let common = REQUIRED
        .iter()
        .filter(|(line, _)| mode == BootMode::SelfTest || !line.contains("self-test:"));
    let own: &[(&str, &str)] = match mode {
        BootMode::SelfTest => &SELFTEST_LINES,
        BootMode::Normal => &NORMAL_LINES,
        BootMode::Refused => &REFUSED_LINES,
    };
    common
        .chain(own)
        .copied()
        .chain([(BOOT_TIME, "a linha de tempo do boot")])
        .collect()
}

/// Os milissegundos de `boot time: kernel N ms to prompt`.
fn kernel_boot_ms(log: &str) -> Option<u64> {
    let rest = &kernel_line(log, BOOT_TIME)?[BOOT_TIME.len()..];
    rest.split_ascii_whitespace().next()?.parse().ok()
}

fn report(arch: Arch, mode: BootMode, outcome: Outcome, log: &str) -> Result<(), String> {
    match outcome {
        Outcome::Booted(elapsed) => {
            if let Some((_, what)) = mode_lines(mode)
                .iter()
                .find(|(line, _)| !has_mark(log, line))
            {
                let reason = format!("boot sem {what} (guidelines 09)");
                return Err(failure(arch, &reason, log));
            }
            if mode == BootMode::SelfTest
                && let Some((_, what)) = USER_OUTPUT
                    .iter()
                    .find(|(line, _)| !has_user_line(log, line))
            {
                let reason = format!("boot sem {what}");
                return Err(failure(arch, &reason, log));
            }
            if mode != BootMode::SelfTest {
                if let Some(line) = normal_forbidden().iter().find(|line| has_mark(log, line)) {
                    let reason = format!("o boot normal rodou `{line}`, que é do modo de teste");
                    return Err(failure(arch, &reason, log));
                }
                let forced = std::env::var("RUSTOS_ACCEL").ok();
                let accel = forced.as_deref().unwrap_or("não confirmada");
                let ceiling = normal_ceiling_ms(forced.as_deref());
                if let Some(ms) = kernel_boot_ms(log).filter(|&ms| ms > ceiling) {
                    let reason = format!(
                        "boot normal levou {ms} ms no kernel, teto de {ceiling} ms com `{accel}`: \
                         autoteste ou medida voltou ao caminho do prompt?"
                    );
                    return Err(failure(arch, &reason, log));
                }
            }
            if mode == BootMode::Refused
                && let Some(problem) = panic_smoke::stack_over_limit(log)
            {
                let reason = format!("pilha da imagem enxuta: {problem} (SDD 15)");
                return Err(failure(arch, &reason, log));
            }
            if mode != BootMode::SelfTest && arch == Arch::Aarch64 {
                let Some(us) = firmware_boot_us(log) else {
                    return Err(failure(arch, "boot sem a linha `boot timeline:`", log));
                };
                if us > FIRMWARE_CEILING_MS * 1000 {
                    let reason = format!(
                        "o firmware levou {} ms até o Limine, teto de {FIRMWARE_CEILING_MS} ms: \
                         a espera do EDK2 voltou? (SDD 05)",
                        us / 1000
                    );
                    return Err(failure(arch, &reason, log));
                }
            }
            let online = cpus_line(Arch::cpus());
            if !has_mark(log, &online) {
                let reason = format!("boot sem `{online}`: nem todos os núcleos subiram");
                return Err(failure(arch, &reason, log));
            }
            if !has_mark(log, KASLR_ON) || has_mark(log, LINK_BASE) {
                return Err(failure(arch, "KASLR desligado: sem `kaslr: on`", log));
            }
            if !cargo::has_feature(&cargo::listed_features(), "debug-addrs")
                && let Some(address) = leak::find_kernel_address(log)
            {
                let reason = format!("endereço do kernel no log: `{address}` (SDD 09)");
                return Err(failure(arch, &reason, log));
            }
            let kernel = kernel_boot_ms(log)
                .map_or_else(String::new, |ms| format!(", kernel {ms} ms até o prompt"));
            println!(
                "xtask: {} {} ok em {} ms (relógio de parede, firmware incluso){kernel}",
                mode.name(),
                arch.name(),
                elapsed.as_millis()
            );
            Ok(())
        }
        Outcome::Panicked => Err(failure(
            arch,
            "o kernel entrou em pânico ou um autoteste falhou",
            log,
        )),
        Outcome::TimedOut => Err(failure(arch, "sem o marcador de boot no tempo limite", log)),
        Outcome::Exited => Err(failure(arch, "a QEMU saiu antes do marcador de boot", log)),
    }
}

#[derive(Clone, Copy, Debug)]
enum Outcome {
    Booted(Duration),
    Panicked,
    TimedOut,
    Exited,
}

fn stream_chunks(mut stdout: impl Read, sender: &mpsc::Sender<String>) {
    let mut buffer = [0u8; 4096];
    while let Ok(read) = stdout.read(&mut buffer) {
        if read == 0
            || sender
                .send(String::from_utf8_lossy(&buffer[..read]).into_owned())
                .is_err()
        {
            break;
        }
    }
}

/// Imprime o que o kernel disse e, depois da marca, o que os programas de usuário disseram: o
/// prefixo `[user]` mostra de quem é cada linha (só as do kernel valem para os gates). As sondas do
/// `handles`, que só provam que um handle escreve, ficam de fora.
fn print_kernel_lines(log: &str) {
    for line in log
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| line.starts_with("[rustos]") || line.starts_with("[user] "))
        .filter(|line| !line.starts_with("[user] probe"))
    {
        println!("  {}", line.trim_end());
    }
}

pub fn failure(arch: Arch, reason: &str, log: &str) -> String {
    let tail: Vec<&str> = log.lines().rev().take(30).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    format!(
        "smoke {}: {reason}\n--- últimas linhas da serial ---\n{}",
        arch.name(),
        tail.join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_are_stripped_from_the_command_output() {
        let raw = "mem\r\n\x1b[2mTotal\x1b[0m 1\r\n";
        assert_eq!(
            strip_escapes(raw),
            "mem\n\x1b[2mTotal".replace("\x1b[2m", "") + " 1\n"
        );
    }

    #[test]
    fn the_prompt_tail_marks_the_end_of_a_command() {
        let prompt = "\x1b[1;38;5;208mrustos\x1b[0m \x1b[34m~\x1b[0m % ";
        assert!(ends_with_prompt(&format!("ps\r\n  0 idle\r\n{prompt}")));
        assert!(!ends_with_prompt("busy 23% \r\n"));
    }

    #[test]
    fn only_the_self_test_mode_asks_for_the_task_self_tests() {
        let wants = |mode, text: &str| mode_lines(mode).iter().any(|(line, _)| line.contains(text));
        assert!(wants(BootMode::SelfTest, "task self-test"));
        assert!(wants(BootMode::SelfTest, "alloc storm"));
        assert!(!wants(BootMode::Normal, "task self-test"));
        assert!(!wants(BootMode::Normal, "smp self-test"));
        for mode in [BootMode::SelfTest, BootMode::Normal] {
            assert!(wants(mode, "w^x: ok"));
            assert!(wants(mode, "boot time:"));
        }
    }

    /// Um log de boot normal que passa em tudo, com o tempo e as linhas extras que o teste pedir.
    fn normal_log(ms: u64, extra: &str) -> String {
        normal_log_with(ms, 100_000, extra)
    }

    fn normal_log_with(ms: u64, firmware_us: u64, extra: &str) -> String {
        use std::fmt::Write as _;
        let mut log = String::new();
        for (line, _) in mode_lines(BootMode::Normal) {
            if line == BOOT_TIME {
                continue;
            }
            log.push_str(line);
            log.push('\n');
        }
        writeln!(log, "{BOOT_TIME}{ms} ms to prompt").unwrap();
        writeln!(
            log,
            "[rustos] boot timeline: firmware {firmware_us} us, bootloader 1 us"
        )
        .unwrap();
        log.push_str(&cpus_line(Arch::cpus()));
        writeln!(log, "\n{KASLR_ON}\n{extra}").unwrap();
        log
    }

    #[test]
    fn the_normal_boot_refuses_every_line_of_the_self_test_mode() {
        let forbidden = normal_forbidden();
        for line in [
            "[rustos] heap self-test: ok",
            "[rustos] timer: ",
            "[rustos] random bench: ",
            "[rustos] zero bench: ",
            "[rustos] smp self-test:",
            "[rustos] wake self-test:",
            "[rustos] task self-test:",
            "[rustos] switch: ",
            "[rustos] initramfs:",
        ] {
            assert!(forbidden.contains(&line), "{line}");
        }
        assert!(!forbidden.contains(&"[rustos] frame self-test: ok"));
        assert!(!forbidden.contains(&"[rustos] exception self-test: ok"));
        for line in [
            "[rustos] user: entered ring3/el0",
            "[rustos] user: init loaded (",
            "[rustos] user: pid 1 exited (code 0,",
        ] {
            assert!(forbidden.contains(&line), "{line}");
        }
        assert!(
            report(
                Arch::X86_64,
                BootMode::Normal,
                Outcome::Booted(Duration::ZERO),
                &normal_log(5, "")
            )
            .is_ok()
        );
        for line in [
            "[rustos] heap self-test: ok",
            "[rustos] timer: 100 Hz",
            "[rustos] initramfs: not mapped, the boot goes on without it",
            "[rustos] initramfs: 8 files, 272 bytes",
            "[rustos] user: fault contained (pid 3)",
            "[rustos] user self-test: ok",
        ] {
            let log = normal_log(5, line);
            let outcome = Outcome::Booted(Duration::ZERO);
            let result = report(Arch::X86_64, BootMode::Normal, outcome, &log);
            assert!(result.unwrap_err().contains("do modo de teste"), "{line}");
        }
    }

    #[test]
    fn the_refused_boot_wants_both_refusals_and_none_of_the_self_test_lines() {
        let wants = |text: &str| {
            mode_lines(BootMode::Refused)
                .iter()
                .any(|(line, _)| line.contains(text))
        };
        assert!(wants("selftest: not in this build") && wants("bench: not in this build"));
        assert!(!wants("task self-test") && !wants("alloc storm"));
        assert_eq!(BootMode::Refused.cmdline(), Some("rustos.selftest=1"));
        let outcome = Outcome::Booted(Duration::ZERO);
        let mut log = normal_log(5, "");
        let missing = report(Arch::X86_64, BootMode::Refused, outcome, &log);
        assert!(missing.unwrap_err().contains("recusa"));
        log.push_str(SELFTEST_REFUSED);
        log.push('\n');
        log.push_str(BENCH_REFUSED);
        log.push('\n');
        let no_ps = report(Arch::X86_64, BootMode::Refused, outcome, &log);
        assert!(no_ps.unwrap_err().contains("coluna de pilha"));
        log.push_str("  0  idle  0 run  1.7  479  1.1/16K  -\n");
        assert!(report(Arch::X86_64, BootMode::Refused, outcome, &log).is_ok());
        let deep = log.replace("1.1/16K", "13.0/16K");
        let result = report(Arch::X86_64, BootMode::Refused, outcome, &deep);
        assert!(result.unwrap_err().contains("75%"));
        log.push_str("[rustos] heap self-test: ok\n");
        let ran = report(Arch::X86_64, BootMode::Refused, outcome, &log);
        assert!(ran.unwrap_err().contains("do modo de teste"));
    }

    #[test]
    fn only_the_self_test_mode_wants_the_init_and_its_output() {
        let wants = |mode, text: &str| mode_lines(mode).iter().any(|(line, _)| line.contains(text));
        assert!(wants(BootMode::SelfTest, "init loaded"));
        assert!(wants(BootMode::SelfTest, "pid 1 exited (code 0,"));
        assert!(!wants(BootMode::Normal, "init loaded"));
        assert!(!wants(BootMode::Refused, "init loaded"));
        let outcome = Outcome::Booted(Duration::ZERO);
        let mut log = String::new();
        for (line, _) in mode_lines(BootMode::SelfTest) {
            log.push_str(line);
            log.push('\n');
        }
        let result = report(Arch::X86_64, BootMode::SelfTest, outcome, &log);
        assert!(result.unwrap_err().contains("canal de boot"));
    }

    #[test]
    fn the_ps_check_wants_the_count_task_with_a_numeric_pid() {
        let ps = "id  name       pid cpu state\n  1  shell        -   0 run\n  9  count         3   1 wait\n";
        assert!(ps_shows_a_process(ps));
        assert!(!ps_shows_a_process("  1  shell        -   0 run\n"));
        assert!(!ps_shows_a_process("  9  count        -   1 wait\n"));
        assert!(!ps_shows_a_process("  9  user         3   1 wait\n"));
    }

    #[test]
    fn the_ps_task_table_must_be_whole() {
        let whole = "  id  name  pid cpu state switches\n  1  shell  -  0 run  7\n  9  count  3  1 wait  2\ncpu  load  ipi out/in\n  0  1\n";
        assert!(ps_table_is_whole(whole));
        let torn = "  id  name  pid cpu state switches\n  1  shell  -  0 run  7\n[user] count: 3/5\n  9  count  3  1 wait  2\ncpu  load  ipi out/in\n";
        assert!(!ps_table_is_whole(torn));
        assert!(ps_table_is_whole(
            "ps\n  id  name  switches\n  1  shell\ncpu  ipi\n[user] late\n"
        ));
        assert!(!ps_table_is_whole("nothing here"));
    }

    #[test]
    fn an_answer_needs_the_echo_and_a_prompt_after_it() {
        let prompt = "rustos \x1b[34m~\x1b[0m % \x1b[J";
        let redraw = format!("\r\x1b[J[user] count: 1/5\n{prompt}");
        assert!(!answered(&redraw, "ps"));
        assert!(!answered(&format!("{redraw}ps\r\n  1 shell\n"), "ps"));
        assert!(answered(
            &format!("{redraw}ps\r\n  1 shell\n{prompt}"),
            "ps"
        ));
        assert!(!answered(&format!("{redraw}ls\r\n{prompt}"), "s"));
    }

    #[test]
    fn a_redraw_in_the_middle_of_typing_does_not_hide_the_echo() {
        let prompt = "rustos \x1b[34m~\x1b[0m % ";
        let typed = format!(
            "r\r\x1b[J[rustos] user: pid 4 exited\n{prompt}r\x1b[Jun hello\r\nexit code 0\n"
        );
        assert!(!answered(&typed, "run hello"));
        assert!(answered(&format!("{typed}{prompt}\x1b[J"), "run hello"));
    }

    #[test]
    fn a_line_is_what_the_terminal_shows_after_the_last_cr_and_the_erase() {
        assert_eq!(shown("[user] a\r"), "[user] a");
        assert_eq!(shown("PROMPT \r\x1b[J[user] a\r"), "[user] a");
        assert_eq!(shown("\r\x1b[2K\x1b[J[rustos] x"), "[rustos] x");
        assert!(has_user_line(
            "PROMPT\r\x1b[J[user] count: done\r\n",
            "[user] count: done"
        ));
        assert!(!has_user_line(
            "PROMPT [user] count: done\r\n",
            "[user] count: done"
        ));
        assert_eq!(
            kernel_lines("PROMPT\r\x1b[J[rustos] user: pid 4 exited\n").count(),
            1
        );
    }

    #[test]
    fn a_prompt_glued_to_user_output_is_caught_and_a_redraw_is_not() {
        let prompt = "rustos \x1b[34m~\x1b[0m % ";
        assert!(check_no_prompt_glue(&format!("{prompt}[user] count: 1/5\r\n")).is_err());
        assert!(check_no_prompt_glue(&format!("{prompt}\r\x1b[K[user] count: 1/5\r\n")).is_ok());
        assert!(check_no_prompt_glue(&format!("{prompt}ps\r\n")).is_ok());
    }

    #[test]
    fn the_heap_line_of_mem_is_read_as_the_used_amount() {
        let mem = "Memory\n  usable   249.3 MiB\n  heap           18.1 KiB  (11 pages, 44.0 KiB held)\n  kernel   340.0 KiB\n";
        assert_eq!(heap_used(mem).as_deref(), Some("18.1 KiB"));
        assert_eq!(
            heap_used("  heap  512 B  (1 pages)").as_deref(),
            Some("512 B")
        );
        assert_eq!(heap_used("sem heap aqui"), None);
        let leaked = mem.replace("18.1 KiB  (", "23.9 KiB  (");
        assert_ne!(heap_used(mem), heap_used(&leaked));
    }

    #[test]
    fn the_whole_normal_log_keeps_the_self_test_lines_out_but_lets_the_session_launch_processes() {
        let session = SESSION_OWN.join("\n");
        assert!(check_session_forbidden(BootMode::Normal, &session).is_ok());
        for line in [
            "[rustos] user: init loaded (3 segments)",
            "[rustos] user self-test: ok",
            "[rustos] heap self-test: ok",
            "[rustos] initramfs: 8 files, 1 bytes",
            "[rustos] switch: 1 ns",
        ] {
            let log = format!("{session}\n{line}\n");
            let result = check_session_forbidden(BootMode::Normal, &log);
            assert!(result.unwrap_err().contains("do modo de teste"), "{line}");
        }
        // O modo de teste roda tudo isso: a conferência é só do normal.
        assert!(check_session_forbidden(BootMode::SelfTest, "[rustos] heap self-test: ok").is_ok());
    }

    #[test]
    fn the_session_has_no_command_without_an_expectation() {
        for step in &SESSION {
            assert!(!step.expect.is_empty(), "{}", step.command);
        }
        let commands: Vec<&str> = SESSION.iter().map(|step| step.command).collect();
        assert!(commands.contains(&"run hello") && commands.contains(&"run crash"));
        assert!(commands.contains(&"ls") && commands.contains(&"ps"));
    }

    #[test]
    fn the_init_output_counts_only_as_a_user_line() {
        let ok = USER_OUTPUT[1].0;
        assert!(has_user_line("[user] init: ok\r\n", ok));
        assert!(!has_user_line("[rustos] [user] init: ok\n", ok));
        assert!(!has_user_line("[user] init: FAILED clock\n", ok));
        assert!(!has_user_line(
            "[user] handles: FAILED 1 of 9 assertions\n",
            USER_OUTPUT[2].0
        ));
    }

    #[test]
    fn the_self_test_image_expects_one_file_per_program() {
        let files = crate::user::programs_for(true).len();
        let (line, _) = SELFTEST_LINES
            .iter()
            .find(|(line, _)| line.starts_with("[rustos] initramfs:"))
            .unwrap();
        assert!(line.contains(&format!("{files} files")), "{line}");
    }

    #[test]
    fn the_normal_boot_has_a_time_ceiling() {
        assert_eq!(normal_ceiling_ms(None), 500);
        assert_eq!(normal_ceiling_ms(Some("tcg")), 500);
        assert_eq!(normal_ceiling_ms(Some("hvf")), 50);
        assert_eq!(normal_ceiling_ms(Some("kvm")), 50);
        let slow = normal_log(99_999, "");
        let outcome = Outcome::Booted(Duration::ZERO);
        let result = report(Arch::Aarch64, BootMode::Normal, outcome, &slow);
        assert!(result.unwrap_err().contains("teto de"));
        let result = report(Arch::Aarch64, BootMode::SelfTest, outcome, &slow);
        assert!(
            result.is_err(),
            "o modo de teste segue exigindo as linhas dele"
        );
    }

    #[test]
    fn the_aarch64_normal_boot_refuses_a_slow_firmware() {
        let outcome = Outcome::Booted(Duration::ZERO);
        let slow = normal_log_with(2, 5_100_000, "");
        let result = report(Arch::Aarch64, BootMode::Normal, outcome, &slow);
        assert!(result.unwrap_err().contains("espera do EDK2"));
        let fast = normal_log_with(2, 3_999_000, "");
        assert!(report(Arch::Aarch64, BootMode::Normal, outcome, &fast).is_ok());
        let missing = normal_log(2, "").replace("boot timeline", "boot tl");
        let result = report(Arch::Aarch64, BootMode::Normal, outcome, &missing);
        assert!(result.unwrap_err().contains("boot timeline"));
        assert_eq!(firmware_boot_us(&slow), Some(5_100_000));
        assert!(report(Arch::X86_64, BootMode::Normal, outcome, &slow).is_ok());
    }

    #[test]
    fn a_fatal_line_is_read_to_its_end_before_the_report() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send("[rustos] interrupts: Missing".to_owned())
            .unwrap();
        let late = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            sender.send("Feature(\"pit\")\n".to_owned()).unwrap();
            sender
        });
        let (outcome, log) = wait_for_boot(&receiver, Instant::now());
        assert!(matches!(outcome, Outcome::Panicked));
        assert!(log.ends_with("Feature(\"pit\")\n"), "{log}");
        drop(late.join());
    }

    #[test]
    fn the_kernel_boot_time_is_read_from_its_line() {
        let log = "[rustos] boot time: kernel 41 ms to prompt (memory 3, smp 0)\n";
        assert_eq!(kernel_boot_ms(log), Some(41));
        assert_eq!(kernel_boot_ms("[rustos] boot time: kernel soon"), None);
        assert_eq!(kernel_boot_ms("nada"), None);
    }

    #[test]
    fn the_smoke_line_follows_the_requested_core_count() {
        assert_eq!(cpus_line(4), "[rustos] cpus: 4 online");
        assert_eq!(cpus_line(12), "[rustos] cpus: 8 online");
    }

    #[test]
    fn a_user_line_that_imitates_the_boot_marker_does_not_end_the_boot() {
        let (tx, rx) = mpsc::channel();
        tx.send("[rustos] user: entered ring3/el0\r\n[user] [rustos] boot ok\r\n".to_owned())
            .unwrap();
        tx.send("[user] x [rustos] panic: fake FAILED\r\n".to_owned())
            .unwrap();
        drop(tx);
        let (outcome, _) = wait_for_boot(&rx, Instant::now());
        assert!(matches!(outcome, Outcome::Exited), "{outcome:?}");
    }

    #[test]
    fn the_real_marker_still_ends_the_boot_and_a_carriage_return_is_ignored() {
        let (tx, rx) = mpsc::channel();
        tx.send("[user] [rustos] boot ok\r\n[rustos] boot ok\r\n".to_owned())
            .unwrap();
        let (outcome, _) = wait_for_boot(&rx, Instant::now());
        assert!(matches!(outcome, Outcome::Booted(_)), "{outcome:?}");
    }

    #[test]
    fn marks_match_only_kernel_lines_and_the_kernel_line_keeps_its_prefix() {
        let log = "[user] [rustos] timer: 100 Hz\r\nlixo [rustos] frame self-test: ok\n[rustos] cpus: 4 online (found 4)\r\n";
        assert!(!has_mark(log, "[rustos] timer: "));
        assert!(!has_mark(log, "[rustos] frame self-test: ok"));
        assert!(has_mark(log, "[rustos] cpus: 4 online"));
        assert!(!has_fatal(log));
        assert!(!has_fatal("[user] FAILED\n"));
        assert!(has_fatal("[rustos] w^x: FAILED\n"));
        let boot =
            "[user] [rustos] boot time: kernel 1 ms\n[rustos] boot time: kernel 25 ms to prompt\n";
        assert_eq!(kernel_boot_ms(boot), Some(25));
    }
}
