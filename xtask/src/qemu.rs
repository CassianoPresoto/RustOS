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
use crate::{image, leak, png, workspace};

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
const REQUIRED: [(&str, &str); 5] = [
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
];
const SMOKE_TIMEOUT: Duration = Duration::from_secs(120);

/// Como o `smoke` sobe o kernel: com os autotestes e os benchmarks, ou só o essencial (SDD 06).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BootMode {
    /// `rustos.selftest=1` na linha de comando: o que o `make smoke` confere.
    SelfTest,
    /// Sem linha de comando: o boot que o usuário vê, o do `make smoke-fast`.
    Normal,
}

impl BootMode {
    const fn cmdline(self) -> Option<&'static str> {
        match self {
            Self::SelfTest => Some("rustos.selftest=1"),
            Self::Normal => None,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::SelfTest => "smoke",
            Self::Normal => "smoke-fast",
        }
    }
}

/// O que só o modo de teste imprime: se uma destas faltar, a linha de comando se perdeu e o
/// boot que passou foi o normal.
const SELFTEST_LINES: [(&str, &str); 11] = [
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
];
/// O que o boot normal confere, além do que o `REQUIRED` tem de comum aos dois modos.
const NORMAL_LINES: [(&str, &str); 2] = [
    ("[rustos] frame self-test: ok", "o autoteste de frame"),
    ("[rustos] exception self-test: ok", "o autoteste de exceção"),
];
/// A linha de tempo do boot, nos dois modos (SDD 04).
const BOOT_TIME: &str = "[rustos] boot time: kernel ";
/// Só o modo normal tem de faltar com estas: autoteste e medida que ficam para o modo de teste.
const NORMAL_FORBIDDEN: [&str; 2] = ["[rustos] task self-test:", "[rustos] switch: "];

/// A linha em que o kernel diz quantos núcleos subiram; o limite do kernel (8) corta o pedido.
fn cpus_line(requested: usize) -> String {
    format!("[rustos] cpus: {} online", requested.min(MAX_KERNEL_CPUS))
}

/// Igual a `MAX_CPUS` de `core/hal`; o xtask não depende de crate do workspace.
const MAX_KERNEL_CPUS: usize = 8;

fn command(root: &Path, arch: Arch, profile: Profile) -> Result<Command, String> {
    command_with(root, arch, profile, &cargo::features(), None)
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

pub fn smoke(root: &Path, arch: Arch, profile: Profile, mode: BootMode) -> Result<(), String> {
    let mut qemu = command_with(root, arch, profile, &cargo::features(), mode.cmdline())?;
    qemu.args(["-display", "none", "-serial", "stdio", "-monitor", "none"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let started = Instant::now();
    let mut child = qemu
        .spawn()
        .map_err(|error| format!("não consegui rodar {}: {error}", arch.qemu()))?;
    let receiver = stream_stdout(&mut child)?;
    let (outcome, log) = wait_for_boot(&receiver, started);
    child.kill().ok();
    child.wait().ok();

    print_kernel_lines(&log);
    report(arch, mode, outcome, &log)
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
fn wait_for_prompt(receiver: &mpsc::Receiver<String>) {
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let mut seen = String::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match receiver.recv_timeout(left) {
            Ok(chunk) => seen.push_str(&chunk),
            Err(_) => return,
        }
        if ends_with_prompt(&seen) {
            return;
        }
    }
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
            wait_for_prompt(receiver);
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

fn wait_for_boot(receiver: &mpsc::Receiver<String>, started: Instant) -> (Outcome, String) {
    let mut log = String::new();
    let outcome = loop {
        let remaining = SMOKE_TIMEOUT.saturating_sub(started.elapsed());
        match receiver.recv_timeout(remaining) {
            Ok(chunk) => log.push_str(&chunk),
            Err(mpsc::RecvTimeoutError::Timeout) => break Outcome::TimedOut,
            Err(mpsc::RecvTimeoutError::Disconnected) => break Outcome::Exited,
        }
        if log.contains(BOOT_OK) {
            break Outcome::Booted(started.elapsed());
        }
        if log.contains(PANIC) || FATAL.iter().any(|mark| log.contains(mark)) {
            break Outcome::Panicked;
        }
    };
    (outcome, log)
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
    };
    common
        .chain(own)
        .copied()
        .chain([(BOOT_TIME, "a linha de tempo do boot")])
        .collect()
}

/// Os milissegundos de `boot time: kernel N ms to prompt`.
fn kernel_boot_ms(log: &str) -> Option<u64> {
    let rest = &log[log.find(BOOT_TIME)? + BOOT_TIME.len()..];
    rest.split_ascii_whitespace().next()?.parse().ok()
}

fn report(arch: Arch, mode: BootMode, outcome: Outcome, log: &str) -> Result<(), String> {
    match outcome {
        Outcome::Booted(elapsed) => {
            if let Some((_, what)) = mode_lines(mode)
                .iter()
                .find(|(line, _)| !log.contains(line))
            {
                let reason = format!("boot sem {what} (guidelines 09)");
                return Err(failure(arch, &reason, log));
            }
            if mode == BootMode::Normal
                && let Some(line) = NORMAL_FORBIDDEN.iter().find(|line| log.contains(*line))
            {
                let reason = format!("o boot normal rodou `{line}`, que é do modo de teste");
                return Err(failure(arch, &reason, log));
            }
            let online = cpus_line(Arch::cpus());
            if !log.contains(&online) {
                let reason = format!("boot sem `{online}`: nem todos os núcleos subiram");
                return Err(failure(arch, &reason, log));
            }
            if !log.contains(KASLR_ON) || log.contains(LINK_BASE) {
                return Err(failure(arch, "KASLR desligado: sem `kaslr: on`", log));
            }
            if !cargo::features().iter().any(|name| name == "debug-addrs")
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

#[derive(Clone, Copy)]
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
    for line in log
        .lines()
        .filter_map(|line| line.find("[rustos]").map(|at| &line[at..]))
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

    #[test]
    fn the_kernel_boot_time_is_read_from_its_line() {
        let log = "x [rustos] boot time: kernel 41 ms to prompt (memory 3, smp 0)\n";
        assert_eq!(kernel_boot_ms(log), Some(41));
        assert_eq!(kernel_boot_ms("[rustos] boot time: kernel soon"), None);
        assert_eq!(kernel_boot_ms("nada"), None);
    }

    #[test]
    fn the_smoke_line_follows_the_requested_core_count() {
        assert_eq!(cpus_line(4), "[rustos] cpus: 4 online");
        assert_eq!(cpus_line(12), "[rustos] cpus: 8 online");
    }
}
