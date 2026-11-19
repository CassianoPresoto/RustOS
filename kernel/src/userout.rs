//! A saída `[user]` dos programas e o shell (SDD 16, "Saída de segundo plano"). Com o shell
//! escrevendo ou no prompt, a saída espera num buffer limitado e o shell a mostra sem rasgar a
//! linha dele; com o shell dormindo à espera de um `run`, ela vai direto à tela.

use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

use rustos_shell::Backlog;

use crate::addr;
use crate::console::{self, SerialSink, Sink};
use crate::panic;
use crate::sync::IrqLock;
use crate::task;

/// Bytes de saída de usuário que esperam o shell; o que não cabe é descartado e contado.
const BACKLOG_BYTES: usize = 1024;

struct Gate {
    /// O laço do shell assumiu e o shell não dorme à espera de um processo: a saída espera a fila.
    /// Falso (tudo zero, no `.bss`) até `queued`: antes do shell ninguém desenharia a fila.
    queued: bool,
    backlog: Backlog<BACKLOG_BYTES>,
}

static GATE: IrqLock<Gate> = IrqLock::new(Gate {
    queued: false,
    backlog: Backlog::new(),
});
/// Há o que mostrar na fila: o que acorda o laço do shell sem pegar o lock.
static PENDING: AtomicBool = AtomicBool::new(false);

/// Uma escrita de usuário, já saneada: entra inteira, na tela ou na fila.
pub fn write(args: fmt::Arguments<'_>) {
    if panic::elsewhere() {
        return;
    }
    let mut gate = GATE.lock();
    if !gate.queued {
        console::print(args);
        return;
    }
    gate.backlog.push_with(|out| {
        addr::scrubbed(out, |text| {
            text.write_fmt(args).ok();
        });
        Ok(())
    });
    PENDING.store(true, Ordering::SeqCst);
    drop(gate);
    task::notify_input();
}

/// Uma linha de diagnóstico só da serial (`kprintln_serial!`): no prompt, espera a vez do shell
/// marcada com `SERIAL_ONLY`, e o shell a escreve só na serial.
pub fn write_serial(args: fmt::Arguments<'_>) {
    if panic::elsewhere() {
        return;
    }
    let mut gate = GATE.lock();
    if !gate.queued {
        console::print_serial_now(args);
        return;
    }
    gate.backlog.push_with(|out| {
        out.write_char(SERIAL_ONLY)?;
        addr::scrubbed(out, |text| {
            text.write_fmt(args).ok();
        });
        Ok(())
    });
    PENDING.store(true, Ordering::SeqCst);
    drop(gate);
    task::notify_input();
}

/// Marca, na fila, o texto que vai só à serial, até o fim da linha.
const SERIAL_ONLY: char = '\u{1}';

/// Escreve o que a fila guardou: na serial e na tela, menos o texto marcado `SERIAL_ONLY`.
#[derive(Debug, Default)]
pub struct Drain {
    serial_only: bool,
}

impl fmt::Write for Drain {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let mut rest = text;
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix(SERIAL_ONLY) {
                (self.serial_only, rest) = (true, after);
                continue;
            }
            let end = rest.find(['\n', SERIAL_ONLY]).map_or(rest.len(), |at| {
                at + usize::from(rest.as_bytes()[at] == b'\n')
            });
            let (piece, tail) = rest.split_at(end);
            if self.serial_only {
                SerialSink.write_str(piece)?;
            } else {
                Sink.write_str(piece)?;
            }
            self.serial_only &= !piece.ends_with('\n');
            rest = tail;
        }
        Ok(())
    }
}

/// A fila tem o que o laço do shell deve mostrar.
pub fn pending() -> bool {
    PENDING.load(Ordering::SeqCst)
}

/// O laço do shell assume: a saída de usuário passa a esperar a vez dele.
pub fn queued() {
    GATE.lock().queued = true;
}

/// O shell vai dormir à espera de um processo, com o cursor na coluna 0: mostra o que esperava e
/// deixa a saída passar direto.
pub fn direct() {
    let mut gate = GATE.lock();
    gate.queued = false;
    if !gate.backlog.is_empty() {
        gate.backlog.drain(&mut Drain::default()).ok();
        PENDING.store(false, Ordering::SeqCst);
        console::flush();
    }
}

/// Entrega a fila a `show`, que a escreve de modo a não rasgar o prompt, e a esvazia.
pub fn interject(show: impl FnOnce(&mut Backlog<BACKLOG_BYTES>)) {
    let mut gate = GATE.lock();
    show(&mut gate.backlog);
    PENDING.store(false, Ordering::SeqCst);
}
