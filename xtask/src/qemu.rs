//! Sobe o kernel na QEMU: interativo (`run`) ou como teste de fumaça (`smoke`).

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::arch::Arch;
use crate::cargo::Profile;
use crate::{image, workspace};

/// Igual ao `BOOT_OK_MARKER` do kernel; os dois mudam juntos.
const BOOT_OK: &str = "[rustos] boot ok";
const PANIC: &str = "[rustos] panic";
const SMOKE_TIMEOUT: Duration = Duration::from_secs(120);

fn command(root: &Path, arch: Arch, profile: Profile) -> Result<Command, String> {
    let esp = image::assemble(root, arch, profile)?;
    let firmware = arch.firmware()?;
    let mut qemu = Command::new(arch.qemu());
    qemu.args(arch.qemu_machine_args(&firmware.to_string_lossy(), &esp.to_string_lossy()));
    qemu.args(["-display", "none", "-no-reboot"]);
    Ok(qemu)
}

pub fn run(root: &Path, arch: Arch, profile: Profile) -> Result<(), String> {
    let mut qemu = command(root, arch, profile)?;
    println!("xtask: {} na QEMU; Ctrl-A X encerra", arch.name());
    workspace::run(qemu.args(["-serial", "mon:stdio"]))
}

pub fn smoke(root: &Path, arch: Arch, profile: Profile) -> Result<(), String> {
    let mut qemu = command(root, arch, profile)?;
    qemu.args(["-serial", "stdio", "-monitor", "none"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let started = Instant::now();
    let mut child = qemu
        .spawn()
        .map_err(|error| format!("não consegui rodar {}: {error}", arch.qemu()))?;
    let stdout = child.stdout.take().ok_or("QEMU sem stdout")?;
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || stream_chunks(stdout, &sender));

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
        if log.contains(PANIC) {
            break Outcome::Panicked;
        }
    };
    child.kill().ok();
    child.wait().ok();

    print_kernel_lines(&log);
    match outcome {
        Outcome::Booted(elapsed) => {
            println!(
                "xtask: smoke {} ok em {} ms (relógio de parede, firmware incluso)",
                arch.name(),
                elapsed.as_millis()
            );
            Ok(())
        }
        Outcome::Panicked => Err(format!("smoke {}: o kernel entrou em pânico", arch.name())),
        Outcome::TimedOut => Err(failure(
            arch,
            "sem o marcador de boot no tempo limite",
            &log,
        )),
        Outcome::Exited => Err(failure(arch, "a QEMU saiu antes do marcador de boot", &log)),
    }
}

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
