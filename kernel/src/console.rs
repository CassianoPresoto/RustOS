//! Console global do kernel e a macro `kprintln!`.

use core::fmt::{self, Write};

use rustos_hal::Console;
use rustos_sync::SpinLock;

use crate::platform::PlatformConsole;

static CONSOLE: SpinLock<Option<PlatformConsole>> = SpinLock::new(None);

pub fn install(console: PlatformConsole) {
    *CONSOLE.lock() = Some(console);
}

pub fn print(args: fmt::Arguments<'_>) {
    if let Some(console) = CONSOLE.lock().as_mut() {
        Writer(console).write_fmt(args).ok();
    }
}

/// Caminho do pânico: não espera o lock, que pode estar com quem entrou em pânico.
pub fn print_panic(args: fmt::Arguments<'_>) {
    if let Some(mut guard) = CONSOLE.try_lock()
        && let Some(console) = guard.as_mut()
    {
        Writer(console).write_fmt(args).ok();
    }
}

struct Writer<'a, C>(&'a mut C);

impl<C: Console> Write for Writer<'_, C> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0.write_bytes(text.as_bytes());
        Ok(())
    }
}

macro_rules! kprintln {
    ($($arg:tt)*) => { $crate::console::print(format_args!("{}\n", format_args!($($arg)*))) };
}
