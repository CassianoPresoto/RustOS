//! Console global do kernel e a macro `kprintln!`: tudo vai para a serial e,
//! quando existe, para a tela.

use core::fmt::{self, Write};

use rustos_hal::{ByteSource, Console, KeyEvent};

use crate::platform::PlatformConsole;
use crate::screen::Screen;
use crate::sync::IrqLock;

/// Locks que mascaram IRQ: o handler da serial pega `SERIAL` dentro da interrupção.
static SERIAL: IrqLock<Option<PlatformConsole>> = IrqLock::new(None);
static SCREEN: IrqLock<Option<Screen>> = IrqLock::new(None);

pub fn install(console: PlatformConsole) {
    *SERIAL.lock() = Some(console);
}

pub fn install_screen(screen: Screen) {
    *SCREEN.lock() = Some(screen);
}

pub fn print(args: fmt::Arguments<'_>) {
    Sink.write_fmt(args).ok();
    flush();
}

/// Desenha na tela o que o `Sink` escreveu: uma vez por mensagem ou tecla, não
/// por pedaço de texto, porque cada desenho repinta linhas inteiras.
pub fn flush() {
    if let Some(screen) = SCREEN.lock().as_mut() {
        screen.render();
    }
}

/// Próximo byte digitado na serial, sem esperar.
pub fn read_byte() -> Option<u8> {
    SERIAL.lock().as_mut().and_then(ByteSource::read_byte)
}

/// Tecla física vira bytes de terminal, pelo layout e pelo modo de cursor da tela.
pub fn key_event(event: KeyEvent, mut push: impl FnMut(u8)) {
    if let Some(screen) = SCREEN.lock().as_mut() {
        screen.key_event(event, &mut push);
    }
}

/// Se há tela: sem ela, as teclas do teclado não viram bytes.
pub fn has_screen() -> bool {
    SCREEN.lock().is_some()
}

/// Troca o layout do teclado; `false` sem tela, onde o teclado não é lido.
pub fn set_layout(layout: rustos_term::Layout) -> bool {
    SCREEN
        .lock()
        .as_mut()
        .map(|screen| screen.set_layout(layout))
        .is_some()
}

/// Caminho do pânico: não espera nenhum lock, que pode estar com quem entrou em pânico.
pub fn print_panic(args: fmt::Arguments<'_>) {
    if let Some(mut guard) = SERIAL.try_lock()
        && let Some(serial) = guard.as_mut()
    {
        Writer(serial).write_fmt(args).ok();
    }
    if let Some(mut guard) = SCREEN.try_lock()
        && let Some(screen) = guard.as_mut()
    {
        ScreenWriter(screen).write_fmt(args).ok();
        screen.render();
    }
}

/// Escreve na serial e na grade da tela, cada uma com o próprio lock, nessa
/// ordem. Quem escreve pelo `Sink` chama `flush` no fim.
#[derive(Debug)]
pub struct Sink;

impl Write for Sink {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if let Some(serial) = SERIAL.lock().as_mut() {
            serial.write_bytes(text.as_bytes());
        }
        if let Some(screen) = SCREEN.lock().as_mut() {
            screen.write(text.as_bytes());
        }
        Ok(())
    }
}

struct Writer<'a, C>(&'a mut C);

impl<C: Console> Write for Writer<'_, C> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0.write_bytes(text.as_bytes());
        Ok(())
    }
}

struct ScreenWriter<'a>(&'a mut Screen);

impl Write for ScreenWriter<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0.write(text.as_bytes());
        Ok(())
    }
}

macro_rules! kprintln {
    ($($arg:tt)*) => { $crate::console::print(format_args!("{}\n", format_args!($($arg)*))) };
}
