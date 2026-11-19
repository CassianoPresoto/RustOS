//! Comandos embutidos: uma tabela de `{ name, summary, run }`.

use core::fmt;

use crate::ansi::{ACCENT, CLEAR_SCREEN, DIM, LOGO, RESET};
use crate::fmtutil::{Bytes, Millis, Percent, Pid, Seconds, StackUse, Uptime};
use crate::history::Entries;
use rustos_hal::PAGE_SIZE;

use crate::host::{Access, FaultTest, Host, KillCause, Listing, RunOutcome, SystemInfo, TaskState};

#[cfg(feature = "bench")]
use crate::bench::run as bench;

/// Sem a feature `bench` o comando existe e diz que não está no build (como o `fault` sem
/// `panic-tests`).
#[cfg(not(feature = "bench"))]
fn bench(ctx: &mut Ctx<'_>) -> fmt::Result {
    ctx.out.write_str("bench: not in this build\r\n")
}

/// O que um comando recebe: seus argumentos (sem o nome) e as portas.
pub(crate) struct Ctx<'a> {
    pub(crate) args: &'a [&'a str],
    pub(crate) host: &'a dyn Host,
    pub(crate) out: &'a mut dyn fmt::Write,
    pub(crate) history: &'a dyn Entries,
}

/// Entrada da tabela de comandos.
pub(crate) struct Command {
    pub(crate) name: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) run: fn(&mut Ctx<'_>) -> fmt::Result,
}

/// Em ordem alfabética: é a ordem em que `help` e o Tab listam.
pub(crate) const COMMANDS: &[Command] = &[
    Command {
        name: "alt",
        summary: "switch to the alternate screen, or back (alt | alt off)",
        run: alt,
    },
    Command {
        name: "bench",
        summary: "measure context switches, IPIs, multi-core scaling and wake-up jitter",
        run: bench,
    },
    Command {
        name: "clear",
        summary: "clear the screen",
        run: clear,
    },
    Command {
        name: "colors",
        summary: "show the 16 ANSI colors and the 256-color palette",
        run: colors,
    },
    Command {
        name: "echo",
        summary: "print the arguments",
        run: echo,
    },
    Command {
        name: "fault",
        summary: "raise an exception to test the fault path (break | page | text | cpu | lock | nested | all | held)",
        run: fault,
    },
    Command {
        name: "fetch",
        summary: "show a summary of the system",
        run: fetch,
    },
    Command {
        name: "help",
        summary: "list the commands",
        run: help,
    },
    Command {
        name: "history",
        summary: "list the command history",
        run: history,
    },
    Command {
        name: "keymap",
        summary: "show or change the keyboard layout (us | br)",
        run: keymap,
    },
    Command {
        name: "lines",
        summary: "draw a table with the DEC line-drawing character set",
        run: lines,
    },
    Command {
        name: "ls",
        summary: "list the files in the initramfs, with their sizes",
        run: ls,
    },
    Command {
        name: "mem",
        summary: "show the physical memory usage",
        run: mem,
    },
    Command {
        name: "ps",
        summary: "list the tasks (and the user process of each), their state and CPU time",
        run: ps,
    },
    Command {
        name: "random",
        summary: "print a random 64-bit number in hex",
        run: random,
    },
    Command {
        name: "run",
        summary: "run a program from the initramfs as a process (run <program> [&] to not wait)",
        run,
    },
    Command {
        name: "uname",
        summary: "print system information (-a -s -r -m)",
        run: uname,
    },
    Command {
        name: "uptime",
        summary: "show how long the system has been running",
        run: uptime,
    },
    Command {
        name: "version",
        summary: "print the system version",
        run: version,
    },
];

pub(crate) fn find(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.name == name)
}

fn clear(ctx: &mut Ctx<'_>) -> fmt::Result {
    ctx.out.write_str(CLEAR_SCREEN)
}

fn echo(ctx: &mut Ctx<'_>) -> fmt::Result {
    for (index, word) in ctx.args.iter().enumerate() {
        if index > 0 {
            ctx.out.write_char(' ')?;
        }
        ctx.out.write_str(word)?;
    }
    ctx.out.write_str("\r\n")
}

fn help(ctx: &mut Ctx<'_>) -> fmt::Result {
    write!(ctx.out, "{DIM}Built-in commands:{RESET}\r\n")?;
    for command in COMMANDS {
        let Command { name, summary, .. } = command;
        write!(ctx.out, "  {ACCENT}{name:<8}{RESET} {summary}\r\n")?;
    }
    Ok(())
}

fn version(ctx: &mut Ctx<'_>) -> fmt::Result {
    let info = ctx.host.info();
    write!(ctx.out, "{} {}\r\n", info.os_name, info.os_version)
}

fn uname(ctx: &mut Ctx<'_>) -> fmt::Result {
    let info = ctx.host.info();
    let (mut name, mut release, mut machine) = (ctx.args.is_empty(), false, false);
    for arg in ctx.args {
        let Some(flags) = arg.strip_prefix('-').filter(|flags| !flags.is_empty()) else {
            return write!(ctx.out, "uname: extra operand '{arg}'\r\n");
        };
        for flag in flags.chars() {
            match flag {
                's' => name = true,
                'r' => release = true,
                'm' => machine = true,
                'a' => (name, release, machine) = (true, true, true),
                _ => return write!(ctx.out, "uname: invalid option -- '{flag}'\r\n"),
            }
        }
    }
    let fields = [
        (name, info.os_name),
        (release, info.os_version),
        (machine, info.arch),
    ];
    let mut first = true;
    for (_, text) in fields.iter().filter(|(selected, _)| *selected) {
        if !first {
            ctx.out.write_char(' ')?;
        }
        first = false;
        ctx.out.write_str(text)?;
    }
    ctx.out.write_str("\r\n")
}

fn fault(ctx: &mut Ctx<'_>) -> fmt::Result {
    let kind = match ctx.args.first().copied() {
        Some("break") => FaultTest::Breakpoint,
        Some("page") => FaultTest::PageFault,
        Some("text") => FaultTest::WriteText,
        Some("cpu") => FaultTest::CpuPanic,
        Some("lock") => FaultTest::ConsoleLock,
        Some("nested") => FaultTest::NestedPanic,
        Some("all") => FaultTest::Simultaneous,
        Some("held") => FaultTest::HeldLock,
        _ => {
            return ctx
                .out
                .write_str("usage: fault break|page|text|cpu|lock|nested|all|held\r\n");
        }
    };
    match kind {
        FaultTest::PageFault => ctx.out.write_str("reading an unmapped page...\r\n")?,
        FaultTest::WriteText => ctx.out.write_str("writing to the kernel text...\r\n")?,
        FaultTest::CpuPanic => ctx
            .out
            .write_str("panicking a task on another core...\r\n")?,
        FaultTest::ConsoleLock => ctx
            .out
            .write_str("faulting with the console locks held...\r\n")?,
        FaultTest::NestedPanic => ctx
            .out
            .write_str("panicking inside the panic message...\r\n")?,
        FaultTest::Simultaneous => ctx
            .out
            .write_str("panicking on every core at once...\r\n")?,
        FaultTest::HeldLock => ctx
            .out
            .write_str("yielding the CPU with a lock held...\r\n")?,
        FaultTest::Breakpoint => {}
    }
    if !ctx.host.fault_test(kind) {
        return ctx.out.write_str("fault: not available yet\r\n");
    }
    match kind {
        FaultTest::Breakpoint => ctx
            .out
            .write_str("breakpoint handled, back in the shell\r\n"),
        FaultTest::PageFault
        | FaultTest::WriteText
        | FaultTest::ConsoleLock
        | FaultTest::NestedPanic
        | FaultTest::Simultaneous
        | FaultTest::HeldLock => ctx
            .out
            .write_str("no fault was raised: the protection is missing\r\n"),
        FaultTest::CpuPanic => ctx.out.write_str("the panic did not stop the system\r\n"),
    }
}

fn keymap(ctx: &mut Ctx<'_>) -> fmt::Result {
    if ctx.host.keymap().is_none() {
        return ctx.out.write_str("keymap: no keyboard\r\n");
    }
    if let Some(&name) = ctx.args.first()
        && !ctx.host.set_keymap(name)
    {
        return write!(ctx.out, "keymap: unknown layout '{name}' (us, br)\r\n");
    }
    write!(
        ctx.out,
        "keymap: {}\r\n",
        ctx.host.keymap().unwrap_or("none")
    )
}

fn uptime(ctx: &mut Ctx<'_>) -> fmt::Result {
    match ctx.host.uptime_ns() {
        Some(ns) => write!(ctx.out, "up {}\r\n", Uptime(ns)),
        None => ctx.out.write_str("uptime: not available yet\r\n"),
    }
}

fn history(ctx: &mut Ctx<'_>) -> fmt::Result {
    for n in 0..ctx.history.count() {
        if let Some((number, line)) = ctx.history.nth_oldest(n) {
            write!(ctx.out, "{number:>5}  {line}\r\n")?;
        }
    }
    Ok(())
}

fn mem(ctx: &mut Ctx<'_>) -> fmt::Result {
    let memory = ctx.host.info().memory;
    let used = memory.usable_bytes.saturating_sub(memory.free_bytes);
    let share = |part| Percent {
        part,
        whole: memory.usable_bytes,
        decimals: 1,
    };
    let out = &mut *ctx.out;
    write!(out, "{DIM}Memory{RESET}\r\n")?;
    write!(
        out,
        "  {:<13}{:>10}\r\n",
        "usable",
        Bytes(memory.usable_bytes)
    )?;
    write!(
        out,
        "  {:<13}{:>10}  ({})\r\n",
        "used",
        Bytes(used),
        share(used)
    )?;
    let free = memory.free_bytes;
    write!(
        out,
        "  {:<13}{:>10}  ({})\r\n",
        "free",
        Bytes(free),
        share(free)
    )?;
    write!(
        out,
        "  {:<13}{:>10}\r\n",
        "reserved",
        Bytes(memory.reserved_bytes)
    )?;
    let metadata = Percent {
        part: memory.frame_metadata_bytes,
        whole: memory.usable_bytes,
        decimals: 3,
    };
    let metadata_bytes = memory.frame_metadata_bytes;
    write!(
        out,
        "  {:<13}{metadata_bytes:>8} B  ({}, {metadata} of RAM)\r\n",
        "frame meta",
        Bytes(metadata_bytes),
    )?;
    write!(
        out,
        "  {:<13}{:>10}  ({} pages, {} held)\r\n",
        "heap",
        Bytes(memory.heap_used_bytes),
        memory.heap_pages,
        Bytes(memory.heap_pages * PAGE_SIZE),
    )?;
    write!(
        out,
        "  {:<13}{:>10}\r\n",
        "kernel",
        Bytes(memory.kernel_bytes)
    )
}

fn ps(ctx: &mut Ctx<'_>) -> fmt::Result {
    write!(
        ctx.out,
        "{DIM}{:>3}  {:<8} {:>4} {:>3} {:<7} {:>10} {:>9} {:>9}  simd{RESET}\r\n",
        "id", "name", "pid", "cpu", "state", "cpu (s)", "switches", "stack"
    )?;
    let mut result = Ok(());
    ctx.host.tasks(&mut |task| {
        let state = match task.state {
            TaskState::Running => "run",
            TaskState::Ready => "ready",
            TaskState::Sleeping => "sleep",
            TaskState::Blocked => "wait",
        };
        let simd = if task.simd { "yes" } else { "-" };
        let stack = StackUse {
            peak: task.stack_peak,
            bytes: task.stack_bytes,
        };
        if result.is_ok() {
            result = write!(
                ctx.out,
                "{:>3}  {:<8} {:>4} {:>3} {:<7} {:>10} {:>9} {:>9}  {simd}\r\n",
                task.id,
                task.name,
                Pid(task.pid),
                task.cpu,
                state,
                Seconds(task.cpu_ns),
                task.switches,
                stack
            );
        }
    });
    result?;
    cpu_table(ctx)
}

fn ls(ctx: &mut Ctx<'_>) -> fmt::Result {
    if let Some(operand) = ctx.args.first() {
        return write!(ctx.out, "ls: extra operand '{operand}'\r\n");
    }
    let mut header = false;
    let mut result = Ok(());
    let listing = ctx.host.programs(&mut |file| {
        if result.is_err() {
            return;
        }
        if !header {
            header = true;
            result = write!(ctx.out, "{DIM}{:>9}  name{RESET}\r\n", "bytes");
        }
        result = result.and_then(|()| write!(ctx.out, "{:>9}  {}\r\n", file.bytes, file.name));
    });
    result?;
    match listing {
        Listing::Done if !header => ctx.out.write_str("ls: the initramfs is empty\r\n"),
        Listing::Done => Ok(()),
        Listing::NoInitramfs => ctx.out.write_str("ls: no initramfs in this boot\r\n"),
        Listing::Unreadable => ctx.out.write_str("ls: the initramfs is unreadable\r\n"),
    }
}

/// O nome do programa, quantas palavras sobram e se pede para não esperar: um `&` no fim da linha,
/// solto ou colado ao nome.
fn run_operands<'a>(args: &[&'a str]) -> (Option<&'a str>, usize, bool) {
    let (words, mut background) = match args {
        [rest @ .., "&"] => (rest, true),
        _ => (args, false),
    };
    let Some((&first, rest)) = words.split_first() else {
        return (None, 0, background);
    };
    let mut name = first;
    if rest.is_empty()
        && let Some(stripped) = first.strip_suffix('&')
    {
        (name, background) = (stripped, true);
    }
    (
        Some(name).filter(|name| !name.is_empty()),
        rest.len(),
        background,
    )
}

fn run(ctx: &mut Ctx<'_>) -> fmt::Result {
    let (name, extra, background) = run_operands(ctx.args);
    let Some(name) = name else {
        return ctx.out.write_str("usage: run <program> [&]\r\n");
    };
    if extra > 0 {
        return ctx
            .out
            .write_str("run: programs take no arguments yet (usage: run <program> [&])\r\n");
    }
    match ctx.host.run_program(name, !background) {
        RunOutcome::Exited { pid, code } => write!(ctx.out, "exit code {code} (pid {pid})\r\n"),
        RunOutcome::Killed { pid, cause } => write!(ctx.out, "killed: {cause} (pid {pid})\r\n"),
        RunOutcome::Started { pid } => write!(ctx.out, "started pid {pid}\r\n"),
        RunOutcome::NotFound => write!(ctx.out, "run: {name}: no such program (try 'ls')\r\n"),
        RunOutcome::NotExecutable => {
            write!(
                ctx.out,
                "run: {name}: not an executable this kernel accepts\r\n"
            )
        }
        RunOutcome::NoMemory => write!(ctx.out, "run: {name}: out of memory\r\n"),
        RunOutcome::NoInitramfs => ctx.out.write_str("run: no initramfs in this boot\r\n"),
        RunOutcome::Failed => write!(ctx.out, "run: {name}: could not start the process\r\n"),
    }
}

impl fmt::Display for KillCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::PageFault { address: 0, .. } => f.write_str("page fault outside the user half"),
            Self::PageFault { address, access } => {
                let what = match access {
                    Access::Read => "read of",
                    Access::Write => "write to",
                    Access::Execute => "instruction fetch at",
                };
                write!(f, "page fault on a {what} {address:#x}")
            }
            Self::IllegalInstruction => f.write_str("illegal instruction"),
            Self::Unaligned => f.write_str("unaligned access"),
            Self::Arithmetic => f.write_str("arithmetic fault"),
            Self::Breakpoint => f.write_str("breakpoint"),
            Self::OutOfMemory => f.write_str("out of memory"),
            Self::Other => f.write_str("exception"),
        }
    }
}

/// Uma linha por CPU: fila, tempo ocupado, trocas, IPIs e tarefas puxadas.
pub(crate) fn cpu_table(ctx: &mut Ctx<'_>) -> fmt::Result {
    let uptime = ctx.host.uptime_ns().unwrap_or(0).max(1);
    let mut header = false;
    let mut result = Ok(());
    ctx.host.cpus(&mut |cpu| {
        if result.is_err() {
            return;
        }
        if !header {
            header = true;
            result = write!(
                ctx.out,
                "{DIM}{:>3}  {:>4} {:>5} {:>9} {:>11} {:>6}{RESET}\r\n",
                "cpu", "load", "busy", "switches", "ipi out/in", "pulled"
            );
        }
        let busy = 100 - (cpu.idle_ns.min(uptime) * 100 / uptime);
        let ipis = [cpu.ipis_sent, cpu.ipis_received];
        result = result.and_then(|()| {
            write!(
                ctx.out,
                "{:>3}  {:>4} {:>4}% {:>9} {:>5}/{:<5} {:>6}\r\n",
                cpu.index, cpu.load, busy, cpu.switches, ipis[0], ipis[1], cpu.pulled
            )
        });
    });
    result
}

fn random(ctx: &mut Ctx<'_>) -> fmt::Result {
    match ctx.host.random_u64() {
        Some(value) => write!(ctx.out, "0x{value:016x}\r\n"),
        None => write!(ctx.out, "random: no generator\r\n"),
    }
}

fn blocks(out: &mut dyn fmt::Write, from: u16, to: u16) -> fmt::Result {
    for color in from..to {
        write!(out, "\x1b[48;5;{color}m  ")?;
    }
    write!(out, "{RESET}\r\n")
}

fn colors(ctx: &mut Ctx<'_>) -> fmt::Result {
    let out = &mut *ctx.out;
    write!(out, "{DIM}ANSI colors (0-15){RESET}\r\n")?;
    blocks(out, 0, 16)?;
    write!(out, "{DIM}Color cube (16-231){RESET}\r\n")?;
    for row in 0..6 {
        blocks(out, 16 + row * 36, 16 + (row + 1) * 36)?;
    }
    write!(out, "{DIM}Grayscale (232-255){RESET}\r\n")?;
    blocks(out, 232, 256)
}

const ALT_ON: &str = "\x1b[?1049h";
const ALT_OFF: &str = "\x1b[?1049l";
const LINE_SET: &str = "\x1b(0";
const ASCII_SET: &str = "\x1b(B";

fn alt(ctx: &mut Ctx<'_>) -> fmt::Result {
    let out = &mut *ctx.out;
    if !ctx.host.has_alternate_screen() {
        return out.write_str("alt: this terminal has no alternate screen\r\n");
    }
    match ctx.args {
        ["off"] => out.write_str(ALT_OFF),
        [] => {
            write!(
                out,
                "{ALT_ON}{CLEAR_SCREEN}{ACCENT}Alternate screen{RESET}\r\n"
            )?;
            write!(
                out,
                "{DIM}`alt off` brings the main screen back.{RESET}\r\n"
            )
        }
        _ => out.write_str("usage: alt [off]\r\n"),
    }
}

/// Uma regra da tabela: `left`, as colunas ligadas por `mid` e `right`, em G0 gráfico.
fn rule(out: &mut dyn fmt::Write, [left, mid, right]: [char; 3], widths: &[usize]) -> fmt::Result {
    write!(out, "{LINE_SET}{left}")?;
    for (index, &width) in widths.iter().enumerate() {
        if index > 0 {
            out.write_char(mid)?;
        }
        for _ in 0..width {
            out.write_char('q')?;
        }
    }
    write!(out, "{right}{ASCII_SET}\r\n")
}

fn lines(ctx: &mut Ctx<'_>) -> fmt::Result {
    const ROWS: [(&str, &str); 4] = [
        ("q", "horizontal"),
        ("x", "vertical"),
        ("l k m j", "corners"),
        ("t u v w n", "tees and cross"),
    ];
    const WIDTHS: [usize; 2] = [11, 18];
    let out = &mut *ctx.out;
    rule(out, ['l', 'w', 'k'], &WIDTHS)?;
    write!(
        out,
        "{LINE_SET}x{ASCII_SET} {ACCENT}{:<9}{RESET} ",
        "ESC ( 0"
    )?;
    write!(
        out,
        "{LINE_SET}x{ASCII_SET} {ACCENT}{:<16}{RESET} ",
        "line drawing"
    )?;
    write!(out, "{LINE_SET}x{ASCII_SET}\r\n")?;
    for (code, meaning) in ROWS {
        rule(out, ['t', 'n', 'u'], &WIDTHS)?;
        write!(
            out,
            "{LINE_SET}x{ASCII_SET} {code:<9} {LINE_SET}x{ASCII_SET} "
        )?;
        write!(out, "{meaning:<16} {LINE_SET}x{ASCII_SET}\r\n")?;
    }
    rule(out, ['m', 'v', 'j'], &WIDTHS)
}

pub(crate) const LOGO_WIDTH: usize = 19;

pub(crate) const LOGO_ROWS: [&str; 9] = [
    "  .-----------.  ",
    " /  #####      \\ ",
    "|  ##   ##      |",
    "|  ##   ##      |",
    "|  #####        |",
    "|  ## ##        |",
    "|  ##  ##       |",
    " \\  ##   ##    / ",
    "  '-----------'  ",
];

fn label(out: &mut dyn fmt::Write, text: &str) -> fmt::Result {
    write!(out, "{ACCENT}{text}{RESET}: ")
}

fn fetch_row(ctx: &mut Ctx<'_>, info: &SystemInfo<'_>, row: usize) -> fmt::Result {
    let uptime = ctx.host.uptime_ns();
    let out = &mut *ctx.out;
    match row {
        0 => {
            label(out, "OS")?;
            out.write_str(info.os_name)
        }
        1 => {
            label(out, "Kernel")?;
            write!(out, "{} {}", info.os_name, info.os_version)
        }
        2 => {
            label(out, "Arch")?;
            out.write_str(info.arch)
        }
        3 => {
            label(out, "Bootloader")?;
            match info.bootloader {
                Some(boot) => write!(out, "{} {}", boot.name, boot.version),
                None => out.write_str("unknown"),
            }
        }
        4 => {
            label(out, "Boot")?;
            match info.boot_timeline {
                Some(line) => write!(
                    out,
                    "firmware {} ms, loader {} ms",
                    Millis(line.firmware_us()),
                    Millis(line.loader_us())
                ),
                None => out.write_str("unknown"),
            }
        }
        5 => {
            label(out, "CPU")?;
            out.write_str(info.cpu.unwrap_or("unknown"))
        }
        6 => {
            let memory = info.memory;
            let used = memory.usable_bytes.saturating_sub(memory.free_bytes);
            let share = Percent {
                part: used,
                whole: memory.usable_bytes,
                decimals: 0,
            };
            label(out, "Memory")?;
            write!(
                out,
                "{} / {} ({share})",
                Bytes(used),
                Bytes(memory.usable_bytes)
            )
        }
        7 => {
            label(out, "Display")?;
            match info.display {
                Some(d) => write!(
                    out,
                    "{}x{} ({}x{} cells)",
                    d.width, d.height, d.cols, d.rows
                ),
                None => out.write_str("none"),
            }
        }
        _ => {
            label(out, "Uptime")?;
            match uptime {
                Some(ns) => write!(out, "{}", Uptime(ns)),
                None => out.write_str("not available yet"),
            }
        }
    }
}

fn fetch(ctx: &mut Ctx<'_>) -> fmt::Result {
    let info = ctx.host.info();
    for (row, logo) in LOGO_ROWS.iter().enumerate() {
        write!(ctx.out, "{LOGO}{logo:<LOGO_WIDTH$}{RESET}")?;
        fetch_row(ctx, &info, row)?;
        ctx.out.write_str("\r\n")?;
    }
    write!(ctx.out, "\r\n{:LOGO_WIDTH$}", "")?;
    blocks(ctx.out, 0, 16)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeHost, info};
    use crate::{Listing, RunOutcome};

    fn run(name: &str, args: &[&str], host: &FakeHost) -> String {
        let mut out = String::new();
        let mut ctx = Ctx {
            args,
            host,
            out: &mut out,
            history: &crate::history::History::<16, 4>::new(),
        };
        (find(name).expect("known command").run)(&mut ctx).unwrap();
        out
    }

    #[test]
    fn table_is_sorted_and_names_are_unique() {
        let names: Vec<_> = COMMANDS.iter().map(|c| c.name).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted);
    }

    #[test]
    fn every_command_has_a_summary_and_help_lists_it() {
        let help = run("help", &[], &FakeHost::new());
        for command in COMMANDS {
            assert_ne!(command.summary.len(), 0);
            assert!(help.contains(command.name), "{}", command.name);
            assert!(help.contains(command.summary), "{}", command.name);
        }
    }

    #[test]
    fn find_matches_whole_names_only() {
        assert!(find("echo").is_some());
        assert!(find("ech").is_none());
        assert!(find("").is_none());
    }

    #[test]
    fn echo_joins_arguments_with_single_spaces() {
        assert_eq!(run("echo", &["a", "b  c"], &FakeHost::new()), "a b  c\r\n");
        assert_eq!(run("echo", &[], &FakeHost::new()), "\r\n");
        assert_eq!(run("echo", &["", "x"], &FakeHost::new()), " x\r\n");
    }

    #[test]
    fn clear_homes_and_erases() {
        assert_eq!(run("clear", &[], &FakeHost::new()), "\x1b[H\x1b[2J");
    }

    #[test]
    fn version_prints_name_and_version() {
        assert_eq!(run("version", &[], &FakeHost::new()), "RustOS 0.1.0\r\n");
    }

    #[test]
    fn uname_defaults_to_the_system_name() {
        assert_eq!(run("uname", &[], &FakeHost::new()), "RustOS\r\n");
    }

    #[test]
    fn uname_flags_select_fields() {
        let host = FakeHost::new();
        assert_eq!(run("uname", &["-s"], &host), "RustOS\r\n");
        assert_eq!(run("uname", &["-r"], &host), "0.1.0\r\n");
        assert_eq!(run("uname", &["-m"], &host), "x86_64\r\n");
        assert_eq!(run("uname", &["-a"], &host), "RustOS 0.1.0 x86_64\r\n");
    }

    #[test]
    fn uname_combines_flags_in_canonical_order() {
        let host = FakeHost::new();
        assert_eq!(run("uname", &["-mr"], &host), "0.1.0 x86_64\r\n");
        assert_eq!(run("uname", &["-m", "-s"], &host), "RustOS x86_64\r\n");
    }

    #[test]
    fn uname_rejects_unknown_flags_and_operands() {
        let host = FakeHost::new();
        assert_eq!(
            run("uname", &["-x"], &host),
            "uname: invalid option -- 'x'\r\n"
        );
        assert_eq!(
            run("uname", &["foo"], &host),
            "uname: extra operand 'foo'\r\n"
        );
        assert_eq!(run("uname", &["-"], &host), "uname: extra operand '-'\r\n");
    }

    #[test]
    fn uptime_reports_the_clock() {
        let mut host = FakeHost::new();
        host.uptime = Some(3_723_000_000_000);
        assert_eq!(run("uptime", &[], &host), "up 1h 02m 03s\r\n");
    }

    #[test]
    fn ps_lists_one_row_per_task_with_state_cpu_and_cpu_time() {
        let out = run("ps", &[], &FakeHost::new());
        let rows: Vec<&str> = out.split("\r\n").collect();
        assert!(
            rows[0].contains("name") && rows[0].contains("switches") && rows[0].contains("cpu")
        );
        assert!(rows[0].contains("stack") && rows[1].contains("3.4/16K"));
        assert!(rows[1].contains("idle") && rows[1].contains("ready") && rows[1].contains("0.250"));
        assert!(rows[2].contains("shell") && rows[2].contains("run") && rows[2].contains("12.345"));
        assert!(
            rows[3].contains("worker") && rows[3].contains("sleep") && rows[3].ends_with("yes")
        );
        // A coluna `cpu` mostra a fila dona: o `FakeHost` põe o id.
        assert!(rows[3].contains("worker") && rows[3].contains("  2 sleep"));
    }

    #[test]
    fn ps_ends_with_one_row_per_cpu() {
        let out = run("ps", &[], &FakeHost::new());
        let rows: Vec<&str> = out.split("\r\n").collect();
        assert!(rows[5].contains("load") && rows[5].contains("ipi out/in"));
        assert!(rows[6].starts_with("  0     2  100%      1000     7/9"));
        assert!(rows[6].ends_with('3'));
        assert!(rows[7].starts_with("  1     1   50%      1001"));
        assert_eq!(rows.len(), 9);
    }

    #[test]
    fn ps_has_a_pid_column_that_is_a_dash_for_kernel_tasks() {
        let out = run("ps", &[], &FakeHost::new());
        let rows: Vec<&str> = out.split("\r\n").collect();
        assert!(rows[0].contains("pid"));
        let pid_of = |row: &str| row.split_whitespace().nth(2).map(str::to_owned);
        assert_eq!(pid_of(rows[1]).as_deref(), Some("-"));
        assert_eq!(pid_of(rows[2]).as_deref(), Some("-"));
        assert!(rows[4].contains("user") && rows[4].contains("wait"));
        assert_eq!(pid_of(rows[4]).as_deref(), Some("43"));
    }

    #[test]
    fn ls_lists_each_file_with_its_size_in_bytes() {
        let out = run("ls", &[], &FakeHost::new());
        let rows: Vec<&str> = out.split("\r\n").collect();
        assert!(rows[0].contains("bytes") && rows[0].contains("name"));
        assert_eq!(rows[1], "    11584  init");
        assert_eq!(rows[2], "     4096  hello");
        assert_eq!(rows.len(), 4);
    }

    #[test]
    fn ls_says_why_there_is_nothing_to_list() {
        let mut host = FakeHost::new();
        host.listing = Listing::NoInitramfs;
        assert_eq!(run("ls", &[], &host), "ls: no initramfs in this boot\r\n");
        host.listing = Listing::Unreadable;
        assert_eq!(run("ls", &[], &host), "ls: the initramfs is unreadable\r\n");
        assert_eq!(run("ls", &["x"], &host), "ls: extra operand 'x'\r\n");
    }

    fn run_with(outcome: RunOutcome, args: &[&str]) -> (String, Option<(String, bool)>) {
        let mut host = FakeHost::new();
        host.outcome = outcome;
        let out = run("run", args, &host);
        let seen = host.last_run.borrow().clone();
        (out, seen)
    }

    #[test]
    fn run_waits_by_default_and_shows_the_exit_code() {
        let (out, seen) = run_with(RunOutcome::Exited { pid: 7, code: 3 }, &["hello"]);
        assert_eq!(out, "exit code 3 (pid 7)\r\n");
        assert_eq!(seen, Some(("hello".to_owned(), true)));
    }

    #[test]
    fn run_with_an_ampersand_does_not_wait_and_names_the_pid() {
        for args in [&["count", "&"][..], &["count&"][..]] {
            let (out, seen) = run_with(RunOutcome::Started { pid: 9 }, args);
            assert_eq!(out, "started pid 9\r\n");
            assert_eq!(seen, Some(("count".to_owned(), false)));
        }
    }

    #[test]
    fn run_shows_the_cause_of_a_death_without_kernel_addresses() {
        let fault = KillCause::PageFault {
            address: 0x6000_0000_1000,
            access: Access::Write,
        };
        let cases = [
            (
                fault,
                "killed: page fault on a write to 0x600000001000 (pid 5)",
            ),
            (
                KillCause::PageFault {
                    address: 0,
                    access: Access::Read,
                },
                "killed: page fault outside the user half (pid 5)",
            ),
            (
                KillCause::IllegalInstruction,
                "killed: illegal instruction (pid 5)",
            ),
            (KillCause::OutOfMemory, "killed: out of memory (pid 5)"),
            (KillCause::Arithmetic, "killed: arithmetic fault (pid 5)"),
        ];
        for (cause, expected) in cases {
            let (out, _) = run_with(RunOutcome::Killed { pid: 5, cause }, &["crash"]);
            assert_eq!(out, format!("{expected}\r\n"));
        }
    }

    #[test]
    fn run_explains_every_way_of_not_starting() {
        let cases = [
            (RunOutcome::NotFound, "run: x: no such program (try 'ls')"),
            (
                RunOutcome::NotExecutable,
                "run: x: not an executable this kernel accepts",
            ),
            (RunOutcome::NoMemory, "run: x: out of memory"),
            (RunOutcome::NoInitramfs, "run: no initramfs in this boot"),
            (RunOutcome::Failed, "run: x: could not start the process"),
        ];
        for (outcome, expected) in cases {
            assert_eq!(run_with(outcome, &["x"]).0, format!("{expected}\r\n"));
        }
    }

    #[test]
    fn run_without_a_program_or_with_arguments_does_not_call_the_host() {
        for args in [&[][..], &["&"][..]] {
            let (out, seen) = run_with(RunOutcome::Failed, args);
            assert_eq!(out, "usage: run <program> [&]\r\n");
            assert_eq!(seen, None);
        }
        let (out, seen) = run_with(RunOutcome::Failed, &["hello", "world"]);
        assert!(out.starts_with("run: programs take no arguments yet"));
        assert_eq!(seen, None);
    }

    #[cfg(feature = "bench")]
    #[test]
    fn bench_prints_both_switch_costs_and_the_jitter_rows() {
        let out = run("bench", &[], &FakeHost::new());
        assert!(out.contains("plain") && out.contains("412 ns"));
        assert!(out.contains("with SIMD") && out.contains("530 ns"));
        assert!(out.contains("200 samples"));
        assert!(out.contains("mean   12.3 us") && out.contains("max   45.6 us"));
        assert!(out.contains("one busy task"));
        assert!(out.contains("pick_next") && out.contains("88 ns"));
        assert!(out.contains("tick") && out.contains("31 ns"));
    }

    #[cfg(feature = "bench")]
    #[test]
    fn bench_prints_the_task_to_task_wake_latency_in_nanoseconds() {
        let out = run("bench", &[], &FakeHost::new());
        assert!(out.contains("task to task"));
        assert!(out.contains("median    84 ns") && out.contains("max    250 ns"));
    }

    #[cfg(feature = "bench")]
    #[test]
    fn bench_leaves_out_the_task_wake_row_when_it_was_not_measured() {
        let mut host = FakeHost::new();
        let mut report = host.bench.unwrap();
        report.wake_task_max_ns = 0;
        host.bench = Some(report);
        assert!(!run("bench", &[], &host).contains("task to task"));
    }

    #[cfg(feature = "bench")]
    #[test]
    fn bench_prints_the_cross_core_switch_ipi_and_scale_rows() {
        let out = run("bench", &[], &FakeHost::new());
        assert!(out.contains("cross-core") && out.contains("3210 ns"));
        assert!(out.contains("IPI round trip") && out.contains("4560 ns"));
        assert!(out.contains("8 CPU-bound tasks"));
        assert!(out.contains("1 cpu ") && out.contains("80.0 ms") && out.contains("1.0x"));
        assert!(out.contains("2 cpus ") && out.contains("40.0 ms") && out.contains("2.0x"));
        assert!(out.contains("4 cpus") && out.contains("20.5 ms") && out.contains("3.9x"));
        assert!(out.contains("ipi out/in"));
        assert!(out.contains("frames locked") && out.contains("90.0 ms") && out.contains("5000"));
        assert!(out.contains("frames cached") && out.contains("9.0 ms"));
        assert!(out.contains("Lock contention") && out.contains("heap") && out.contains("     12"));
    }

    #[cfg(feature = "bench")]
    #[test]
    fn bench_on_one_cpu_leaves_out_the_cross_core_rows() {
        let mut host = FakeHost::new();
        let mut report = host.bench.unwrap();
        report.switch_cross_ns = 0;
        report.scale = [crate::ScaleSample::default(); 3];
        host.bench = Some(report);
        let out = run("bench", &[], &host);
        assert!(!out.contains("cross-core") && !out.contains("Scale"));
        assert!(out.contains("plain"));
    }

    #[cfg(feature = "bench")]
    #[test]
    fn bench_prints_the_zeroing_and_random_rows_when_the_host_measures_them() {
        let out = run("bench", &[], &FakeHost::new());
        assert!(out.contains("4 KiB in 1 MiB") && out.contains("20.5 ns"));
        assert!(out.contains("chacha20") && out.contains("640 MB/s"));
        assert!(out.contains("reseed") && out.contains("next_u64") && out.contains("96.3 ns"));
        let mut host = FakeHost::new();
        host.micro = None;
        assert!(!run("bench", &[], &host).contains("Zeroing"));
    }

    #[cfg(feature = "bench")]
    #[test]
    fn bench_without_tasks_says_so() {
        let mut host = FakeHost::new();
        host.bench = None;
        assert_eq!(run("bench", &[], &host), "bench: no tasks yet\r\n");
    }

    #[test]
    #[cfg(not(feature = "bench"))]
    fn bench_without_its_feature_says_it_is_not_in_the_build() {
        assert_eq!(
            run("bench", &[], &FakeHost::new()),
            "bench: not in this build\r\n"
        );
    }

    #[test]
    fn fault_without_argument_prints_usage() {
        assert!(run("fault", &[], &FakeHost::new()).starts_with("usage: fault"));
        assert!(run("fault", &["disk"], &FakeHost::new()).starts_with("usage: fault"));
    }

    #[test]
    fn fault_reports_when_the_platform_cannot() {
        assert!(run("fault", &["break"], &FakeHost::new()).contains("not available yet"));
    }

    #[test]
    fn fault_text_reports_a_missing_protection_when_it_returns() {
        let mut host = FakeHost::new();
        host.faults = true;
        let output = run("fault", &["text"], &host);
        assert!(output.contains("writing to the kernel text"));
        assert!(output.contains("no fault was raised"));
    }

    #[test]
    fn fault_cpu_says_it_panics_another_core() {
        let mut host = FakeHost::new();
        host.faults = true;
        let output = run("fault", &["cpu"], &host);
        assert!(output.contains("another core"));
        assert!(output.contains("did not stop"));
    }

    #[test]
    fn fault_lock_and_nested_announce_what_they_do_and_report_a_missing_panic() {
        let mut host = FakeHost::new();
        host.faults = true;
        let lock = run("fault", &["lock"], &host);
        assert!(lock.contains("console locks held") && lock.contains("no fault was raised"));
        let nested = run("fault", &["nested"], &host);
        assert!(nested.contains("inside the panic message") && nested.contains("no fault"));
        let all = run("fault", &["all"], &host);
        assert!(all.contains("every core at once") && all.contains("no fault"));
        let held = run("fault", &["held"], &host);
        assert!(held.contains("lock held") && held.contains("no fault"));
        assert!(run("fault", &["lock"], &FakeHost::new()).contains("not available yet"));
    }

    #[test]
    fn fault_break_returns_to_the_shell() {
        let mut host = FakeHost::new();
        host.faults = true;
        assert!(run("fault", &["break"], &host).contains("back in the shell"));
    }

    #[test]
    fn keymap_shows_and_changes_the_layout() {
        let host = FakeHost::new();
        assert_eq!(run("keymap", &[], &host), "keymap: us\r\n");
        assert_eq!(run("keymap", &["br"], &host), "keymap: br\r\n");
        assert!(run("keymap", &["dvorak"], &host).contains("unknown layout 'dvorak'"));
        assert_eq!(run("keymap", &[], &host), "keymap: br\r\n");
    }

    #[test]
    fn keymap_without_keyboard_says_so() {
        let host = FakeHost::new();
        host.keymap.set(None);
        assert_eq!(run("keymap", &["br"], &host), "keymap: no keyboard\r\n");
    }

    #[test]
    fn uptime_without_a_clock_says_so() {
        let mut host = FakeHost::new();
        host.uptime = None;
        assert!(run("uptime", &[], &host).contains("not available yet"));
    }

    #[test]
    fn history_numbers_each_line() {
        let mut log = crate::history::History::<16, 4>::new();
        log.push("ls");
        log.push("echo hi");
        let host = FakeHost::new();
        let mut out = String::new();
        let mut ctx = Ctx {
            args: &[],
            host: &host,
            out: &mut out,
            history: &log,
        };
        history(&mut ctx).unwrap();
        assert_eq!(out, "    1  ls\r\n    2  echo hi\r\n");
    }

    #[test]
    fn history_of_nothing_prints_nothing() {
        assert_eq!(run("history", &[], &FakeHost::new()), "");
    }

    #[test]
    fn mem_reports_every_figure() {
        let text = run("mem", &[], &FakeHost::new());
        assert!(text.contains("usable"), "{text}");
        assert!(text.contains("512.0 MiB"), "{text}");
        assert!(text.contains("free"), "{text}");
        assert!(text.contains("500.0 MiB"), "{text}");
        assert!(text.contains("reserved"), "{text}");
        assert!(text.contains("1.5 MiB"), "{text}");
        assert!(text.contains("16384 B"), "{text}");
        assert!(text.contains("0.003% of RAM"), "{text}");
        assert!(text.contains("2.2 MiB"), "{text}");
    }

    #[test]
    fn mem_reports_the_heap_in_use_and_held() {
        let text = run("mem", &[], &FakeHost::new());
        assert!(text.contains("frame meta"), "{text}");
        assert!(text.contains("heap"), "{text}");
        assert!(text.contains("4.8 KiB"), "{text}");
        assert!(text.contains("12 pages, 48.0 KiB held"), "{text}");
    }

    #[test]
    fn random_prints_the_host_number_as_16_hex_digits() {
        assert_eq!(
            run("random", &[], &FakeHost::new()),
            "0x0123456789abcdef\r\n"
        );
        let mut host = FakeHost::new();
        host.random = Some(0xff);
        assert_eq!(run("random", &[], &host), "0x00000000000000ff\r\n");
    }

    #[test]
    fn random_without_a_generator_says_so() {
        let mut host = FakeHost::new();
        host.random = None;
        assert_eq!(run("random", &[], &host), "random: no generator\r\n");
    }

    #[test]
    fn mem_with_no_memory_does_not_divide_by_zero() {
        let mut host = FakeHost::new();
        host.info.memory = crate::MemoryInfo::default();
        let text = run("mem", &[], &host);
        assert!(text.contains("0 B"), "{text}");
        assert!(text.contains("0.0%"), "{text}");
    }

    #[test]
    fn mem_with_more_free_than_usable_does_not_underflow() {
        let mut host = FakeHost::new();
        host.info.memory.free_bytes = host.info.memory.usable_bytes + 1;
        assert!(run("mem", &[], &host).contains("used"));
    }

    #[test]
    fn colors_paints_all_256_entries() {
        let text = run("colors", &[], &FakeHost::new());
        for color in 0..256 {
            assert!(text.contains(&format!("\x1b[48;5;{color}m")), "{color}");
        }
        assert!(text.contains("ANSI colors"));
    }

    #[test]
    fn lines_draws_a_closed_table_with_the_line_drawing_set() {
        let text = run("lines", &[], &FakeHost::new());
        let plain = text
            .replace("\x1b(0", "")
            .replace("\x1b(B", "")
            .replace("\x1b[0m", "");
        let rows: Vec<&str> = plain.lines().collect();
        let top = format!("l{}w{}k", "q".repeat(11), "q".repeat(18));
        let bottom = top.replace('l', "m").replace('w', "v").replace('k', "j");
        assert_eq!(rows[0], top);
        assert_eq!(*rows.last().unwrap(), bottom);
        assert!(text.matches("\x1b(0").count() >= 10);
        assert_eq!(
            text.matches("\x1b(0").count(),
            text.matches("\x1b(B").count()
        );
    }

    #[test]
    fn alt_enters_the_alternate_screen_and_alt_off_leaves_it() {
        let on = run("alt", &[], &FakeHost::new());
        assert!(on.starts_with("\x1b[?1049h"));
        assert!(on.contains("alt off"));
        assert_eq!(run("alt", &["off"], &FakeHost::new()), "\x1b[?1049l");
        assert!(run("alt", &["x"], &FakeHost::new()).contains("usage"));
    }

    #[test]
    fn alt_warns_instead_of_switching_when_there_is_no_alternate_screen() {
        let mut host = FakeHost::new();
        host.alternate = false;
        for args in [&[][..], &["off"]] {
            let text = run("alt", args, &host);
            assert!(text.contains("no alternate screen"), "{text}");
            assert!(!text.contains("\x1b[?1049"), "{text}");
        }
    }

    #[test]
    fn fetch_shows_every_label_and_value() {
        let text = run("fetch", &[], &FakeHost::new());
        for part in [
            "OS\x1b[0m: RustOS",
            "Kernel\x1b[0m: RustOS 0.1.0",
            "Arch\x1b[0m: x86_64",
            "Bootloader\x1b[0m: Limine 9.2.3",
            "Boot\x1b[0m: firmware 312.4 ms, loader 8.1 ms",
            "CPU\x1b[0m: Test CPU @ 3.0GHz",
            "Memory\x1b[0m: 12.0 MiB / 512.0 MiB (2%)",
            "Display\x1b[0m: 1280x800 (160x50 cells)",
            "Uptime\x1b[0m: 1m 05s",
        ] {
            assert!(text.contains(part), "{part} in {text}");
        }
    }

    #[test]
    fn fetch_ends_with_the_sixteen_color_blocks() {
        let text = run("fetch", &[], &FakeHost::new());
        let last = text.trim_end().lines().last().unwrap();
        assert_eq!(last.matches("\x1b[48;5;").count(), 16);
        assert!(last.contains("\x1b[48;5;15m"));
    }

    #[test]
    fn fetch_falls_back_when_the_system_knows_little() {
        let mut host = FakeHost::new();
        host.uptime = None;
        host.info = crate::SystemInfo {
            bootloader: None,
            boot_timeline: None,
            cpu: None,
            display: None,
            ..info()
        };
        let text = run("fetch", &[], &host);
        assert!(text.contains("Bootloader\x1b[0m: unknown"));
        assert!(text.contains("Boot\x1b[0m: unknown"));
        assert!(text.contains("CPU\x1b[0m: unknown"));
        assert!(text.contains("Display\x1b[0m: none"));
        assert!(text.contains("Uptime\x1b[0m: not available yet"));
    }

    #[test]
    fn fetch_logo_fits_its_column() {
        for row in LOGO_ROWS {
            assert!(row.chars().count() < LOGO_WIDTH, "{row}");
            assert!(row.is_ascii());
        }
    }
}
