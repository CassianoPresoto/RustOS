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
use crate::cargo::Profile;
use crate::{image, png, workspace};

/// Igual ao `BOOT_OK_MARKER` do kernel; os dois mudam juntos.
const BOOT_OK: &str = "[rustos] boot ok";
const PANIC: &str = "[rustos] panic";
/// Linhas que o kernel imprime quando um autoteste ou o boot falha.
const FATAL: [&str; 3] = ["[rustos] fault:", "[rustos] interrupts:", "FAILED"];
/// Endereço em que os linker scripts põem o kernel. Com o KASLR ligado, o Limine
/// carrega em outro lugar e a linha `kernel base:` nunca mostra este (SDD 13).
const LINK_BASE: &str = "kernel base: 0xffffffff80000000 ";
/// Linhas que todo boot precisa ter (guidelines 09): as proteções de hardware, o
/// W^X conferido nas tabelas de página e a memória que o bootloader devolveu.
const REQUIRED: [(&str, &str); 3] = [
    ("[rustos] protections:", "a linha de proteções de hardware"),
    (
        "[rustos] w^x: ok",
        "a conferência de W^X das tabelas de página",
    ),
    ("[rustos] reclaimed:", "a memória devolvida pelo bootloader"),
];
const SMOKE_TIMEOUT: Duration = Duration::from_secs(120);

fn command(root: &Path, arch: Arch, profile: Profile) -> Result<Command, String> {
    let esp = image::assemble(root, arch, profile)?;
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

pub fn smoke(root: &Path, arch: Arch, profile: Profile) -> Result<(), String> {
    let mut qemu = command(root, arch, profile)?;
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
    report(arch, outcome, &log)
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
        Outcome::Booted(_) => capture(&mut child, &socket, &ppm, commands),
        other => report(arch, other, &log).and(Err("sem boot, sem captura".to_owned())),
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

fn capture(
    child: &mut std::process::Child,
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
            stdin
                .write_all(format!("{command}\r").as_bytes())
                .and_then(|()| stdin.flush())
                .map_err(|error| format!("digitar na serial: {error}"))?;
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

fn stream_stdout(child: &mut std::process::Child) -> Result<mpsc::Receiver<String>, String> {
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

fn report(arch: Arch, outcome: Outcome, log: &str) -> Result<(), String> {
    match outcome {
        Outcome::Booted(elapsed) => {
            if let Some((_, what)) = REQUIRED.iter().find(|(line, _)| !log.contains(line)) {
                let reason = format!("boot sem {what} (guidelines 09)");
                return Err(failure(arch, &reason, log));
            }
            if log.contains(LINK_BASE) {
                return Err(failure(
                    arch,
                    "kernel na base do linker: KASLR desligado",
                    log,
                ));
            }
            println!(
                "xtask: smoke {} ok em {} ms (relógio de parede, firmware incluso)",
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

fn failure(arch: Arch, reason: &str, log: &str) -> String {
    let tail: Vec<&str> = log.lines().rev().take(30).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    format!(
        "smoke {}: {reason}\n--- últimas linhas da serial ---\n{}",
        arch.name(),
        tail.join("\n")
    )
}
