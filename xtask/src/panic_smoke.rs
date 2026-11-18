//! `smoke-panic`: derruba o kernel de propósito em cada ponto em que o pânico já foi mudo
//! e confere que a mensagem aparece na serial, sem endereço do kernel (SDD 06).

use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::arch::Arch;
use crate::cargo::{self, Profile};
use crate::{leak, qemu};

/// Feature do kernel que lê `rustos.panic_early` da linha de comando.
const FEATURE: &str = "panic-tests";
/// Tempo máximo de um cenário: firmware, boot (até o shell, se há comando) e a mensagem.
const SCENARIO_TIMEOUT: Duration = Duration::from_secs(120);
/// Depois da última mensagem esperada, o que ainda chega entra na conferência de endereço.
const DRAIN: Duration = Duration::from_millis(500);

struct Scenario {
    name: &'static str,
    /// Linha de comando do Limine; sem ela, o kernel sobe normal.
    cmdline: Option<&'static str>,
    /// Comandos do shell, digitados um a um depois do boot; sem eles, o pânico vem antes do shell.
    commands: &'static [&'static str],
    /// Trechos que têm de aparecer, todos.
    expect: &'static [&'static str],
    /// Trecho que tem de aparecer exatamente uma vez (relatório único de vários pânicos).
    once: Option<&'static str>,
    /// Trechos que não podem aparecer.
    forbid: &'static [&'static str],
    /// Espera o prompt voltar depois do último comando (comando que não derruba o kernel).
    prompt_after: bool,
}

const NONE: Scenario = Scenario {
    name: "",
    cmdline: None,
    commands: &[],
    expect: &[],
    once: None,
    forbid: &[],
    prompt_after: false,
};

const SCENARIOS: [Scenario; 8] = [
    Scenario {
        name: "panic at entry, before memory",
        cmdline: Some("rustos.panic_early=1"),
        expect: &["[rustos] panic:", "panic_early: Entry"],
        ..NONE
    },
    Scenario {
        name: "fault at entry, before the kernel vectors",
        cmdline: Some("rustos.panic_early=2"),
        expect: &[
            "[rustos] fault: page fault at 0x1000 (read, kernel,",
            "at pc kernel+0x",
        ],
        ..NONE
    },
    Scenario {
        name: "panic with the new tables on, before the console",
        cmdline: Some("rustos.panic_early=3"),
        expect: &["[rustos] panic:", "panic_early: Activated"],
        ..NONE
    },
    Scenario {
        name: "fault with the console locks held",
        commands: &["fault lock"],
        expect: &[
            "faulting with the console locks held",
            "[rustos] fault: page fault at ",
        ],
        ..NONE
    },
    Scenario {
        name: "panic inside the panic message",
        commands: &["fault nested"],
        expect: &[
            "panicking inside the panic message",
            "nested: the message of the first panic panicked",
        ],
        ..NONE
    },
    Scenario {
        name: "panic on every core at once, one clean report",
        commands: &["fault all"],
        expect: &[
            "panicking on every core at once",
            "simultaneous panic on cpu",
        ],
        once: Some("[rustos] panic:"),
        forbid: &["failed again", "[[rustos]"],
        ..NONE
    },
    Scenario {
        name: "write to .text shows the offset, not the address",
        commands: &["fault text"],
        expect: &[
            "[rustos] fault: page fault at kernel+0x",
            "(write, kernel, protection)",
        ],
        ..NONE
    },
    Scenario {
        name: "shell output carries no address",
        commands: &["mem", "ps", "fetch"],
        expect: &["idle", "RustOS"],
        prompt_after: true,
        ..NONE
    },
];

pub fn run(root: &Path, arch: Arch, profile: Profile) -> Result<(), String> {
    let mut features = cargo::features();
    if !features.iter().any(|name| name == FEATURE) {
        features.push(FEATURE.to_owned());
    }
    if features.iter().any(|name| name == "debug-addrs") {
        return Err(
            "smoke-panic confere o endereço relativo (`kernel+0x…`): tire `debug-addrs`".to_owned(),
        );
    }
    for scenario in &SCENARIOS {
        let started = Instant::now();
        let log = run_scenario(root, arch, profile, &features, scenario)?;
        for line in log.lines().filter_map(report_line) {
            println!("  {line}");
        }
        println!(
            "xtask: smoke-panic {} ok, {} ({} ms)",
            arch.name(),
            scenario.name,
            started.elapsed().as_millis()
        );
    }
    Ok(())
}

fn run_scenario(
    root: &Path,
    arch: Arch,
    profile: Profile,
    features: &[String],
    scenario: &Scenario,
) -> Result<String, String> {
    let mut command = qemu::command_with(root, arch, profile, features, scenario.cmdline)?;
    command
        .args(["-display", "none", "-serial", "stdio", "-monitor", "none"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = command
        .spawn()
        .map_err(|error| format!("não consegui rodar {}: {error}", arch.qemu()))?;
    let receiver = qemu::stream_stdout(&mut child)?;
    let stdin = child.stdin.take();
    let outcome = observe(&receiver, stdin, scenario);
    child.kill().ok();
    child.wait().ok();
    let log = outcome.log.clone();
    let fail = |reason: &str| {
        Err(qemu::failure(
            arch,
            &format!("{}: {reason}", scenario.name),
            &log,
        ))
    };
    if let Some(problem) = &outcome.problem {
        return fail(problem);
    }
    if let Some(address) = leak::find_kernel_address(&log) {
        return fail(&format!("endereço do kernel na mensagem: `{address}`"));
    }
    Ok(log)
}

/// A linha de pânico ou de falha do kernel, sem o que o firmware e o terminal pintaram antes.
fn report_line(line: &str) -> Option<&str> {
    let at = line
        .find("[rustos] panic")
        .or_else(|| line.find("[rustos] fault"))?;
    Some(line[at..].trim_end())
}

struct Observed {
    log: String,
    /// O que deu errado, se algo deu.
    problem: Option<String>,
}

fn first_missing(log: &str, expect: &[&'static str]) -> Option<&'static str> {
    expect.iter().copied().find(|text| !log.contains(text))
}

/// O que o log tem de errado para o cenário: trecho que falta, que repete ou que não podia vir.
fn verdict(log: &str, scenario: &Scenario) -> Option<String> {
    if let Some(missing) = first_missing(log, scenario.expect) {
        return Some(format!("faltou `{missing}` na serial"));
    }
    if let Some(text) = scenario.once {
        let count = log.matches(text).count();
        if count != 1 {
            return Some(format!("`{text}` apareceu {count} vezes, não uma"));
        }
    }
    scenario
        .forbid
        .iter()
        .find(|text| log.contains(**text))
        .map(|text| format!("apareceu `{text}` na serial"))
}

/// Lê a serial até o cenário se completar (e mais um instante), ou o tempo acabar. Digita
/// cada comando quando o prompt do shell volta depois do anterior.
fn observe(
    receiver: &mpsc::Receiver<String>,
    mut stdin: Option<std::process::ChildStdin>,
    scenario: &Scenario,
) -> Observed {
    let started = Instant::now();
    let mut log = String::new();
    let mut typed = 0;
    // Tamanho do log quando o último comando foi digitado: o prompt tem de vir depois do eco.
    let mut mark = 0;
    let mut complete_at: Option<Instant> = None;
    loop {
        let limit = complete_at.unwrap_or(started + SCENARIO_TIMEOUT);
        let Some(left) = limit.checked_duration_since(Instant::now()) else {
            break;
        };
        match receiver.recv_timeout(left) {
            Ok(chunk) => log.push_str(&chunk),
            Err(_) => break,
        }
        let prompt = qemu::ends_with_prompt(&log) && log.len() > mark;
        if prompt && let Some(command) = scenario.commands.get(typed) {
            let line = format!("{command}\r");
            if let Some(input) = stdin.as_mut() {
                input
                    .write_all(line.as_bytes())
                    .and_then(|()| input.flush())
                    .ok();
            }
            typed += 1;
            mark = log.len() + line.len();
        }
        let all_typed = typed == scenario.commands.len();
        let settled = !scenario.prompt_after || (prompt && all_typed && typed > 0);
        if complete_at.is_none()
            && all_typed
            && settled
            && first_missing(&log, scenario.expect).is_none()
        {
            complete_at = Some(Instant::now() + DRAIN);
        }
    }
    let problem = verdict(&log, scenario);
    Observed { log, problem }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_verdict_wants_one_report_and_none_of_the_forbidden_text() {
        let scenario = Scenario {
            expect: &["panic:"],
            once: Some("[rustos] panic:"),
            forbid: &["failed again"],
            ..NONE
        };
        assert_eq!(verdict("[rustos] panic: x", &scenario), None);
        assert!(verdict("nada", &scenario).unwrap().contains("faltou"));
        let twice = "[[rustos] panic:[[rustos] panic:";
        assert!(verdict(twice, &scenario).unwrap().contains("2 vezes"));
        let again = "[rustos] panic: x failed again";
        assert!(verdict(again, &scenario).unwrap().contains("failed again"));
    }

    #[test]
    fn the_first_missing_text_is_reported() {
        let expect = ["a", "b", "c"];
        assert_eq!(first_missing("abc", &expect), None);
        assert_eq!(first_missing("ac", &expect), Some("b"));
    }

    #[test]
    fn the_report_line_starts_at_the_kernel_tag() {
        assert_eq!(
            report_line("\x1b[2J[rustos] fault: x at pc kernel+0x1\r"),
            Some("[rustos] fault: x at pc kernel+0x1")
        );
        assert_eq!(report_line("[rustos] cpus: 4 online"), None);
    }

    #[test]
    fn every_scenario_expects_something_and_the_shell_ones_name_a_command() {
        for scenario in &SCENARIOS {
            assert!(!scenario.expect.is_empty(), "{}", scenario.name);
            assert_ne!(scenario.cmdline.is_some(), !scenario.commands.is_empty());
        }
    }

    #[test]
    fn the_early_scenarios_use_the_values_the_kernel_parses() {
        let lines: Vec<_> = SCENARIOS.iter().filter_map(|s| s.cmdline).collect();
        assert_eq!(
            lines,
            [
                "rustos.panic_early=1",
                "rustos.panic_early=2",
                "rustos.panic_early=3"
            ]
        );
    }
}
