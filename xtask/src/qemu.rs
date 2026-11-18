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
const REQUIRED: [(&str, &str); 6] = [
    ("[rustos] protections:", "a linha de proteções de hardware"),
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

/// O que só o modo de teste imprime: se uma destas faltar, a linha de comando se perdeu e o
/// boot que passou foi o normal.
const SELFTEST_LINES: [(&str, &str); 16] = [
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
        "[rustos] user: entered ring3/el0",
        "a entrada do primeiro programa em anel 3 ou EL0",
    ),
    (
        "[rustos] user: fault contained (pid ",
        "a falha de usuário que mata só o processo",
    ),
    (
        "[rustos] user self-test: ok",
        "o autoteste dos casos de usuário",
    ),
    (
        "[rustos] initramfs: 1 files, ",
        "a leitura do initramfs (um arquivo de teste)",
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
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    if mode == BootMode::Refused {
        qemu.stdin(Stdio::piped());
    }
    let started = Instant::now();
    let mut child = qemu
        .spawn()
        .map_err(|error| format!("não consegui rodar {}: {error}", arch.qemu()))?;
    let receiver = stream_stdout(&mut child)?;
    let (outcome, mut log) = wait_for_boot(&receiver, started);
    if mode == BootMode::Refused && matches!(outcome, Outcome::Booted(_)) {
        type_lean_commands(&mut child, &receiver, &mut log);
    }
    child.kill().ok();
    child.wait().ok();

    print_kernel_lines(&log);
    report(arch, mode, outcome, &log)
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

/// As linhas do kernel: a linha inteira, sem o `\r`, **começa** com `[rustos]`. Texto de usuário
/// (`debug_write`, sempre prefixado por `[user]`) e lixo de firmware no meio da linha não contam.
fn kernel_lines(log: &str) -> impl Iterator<Item = &str> {
    log.split('\n')
        .map(|line| line.trim_end_matches('\r'))
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

fn print_kernel_lines(log: &str) {
    for line in kernel_lines(log) {
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
            "[rustos] initramfs: 1 files, 272 bytes",
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
