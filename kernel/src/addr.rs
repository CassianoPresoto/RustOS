//! Endereço do kernel no log: deslocamento dentro de uma região, nunca a base (SDD 09).
//! A feature `debug-addrs` devolve o endereço cru.

use core::fmt::{self, Write};
use core::sync::atomic::{AtomicU64, Ordering};

#[cfg(not(feature = "debug-addrs"))]
use rustos_hal::redact::Scrubber;
use rustos_hal::redact::{Region, Regions};

use crate::vm;

static DIRECT: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
static WINDOWS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

/// Guarda o mapa direto, `[início, fim)`, para o log nomear o que cai dentro dele.
pub fn set_direct(range: (u64, u64)) {
    DIRECT[0].store(range.0, Ordering::Relaxed);
    DIRECT[1].store(range.1.saturating_sub(range.0), Ordering::Relaxed);
}

/// Guarda as janelas de dispositivos e de pilhas: `len` bytes cada, uma depois da outra.
/// A de módulos vem depois delas e não tem região no log.
pub fn set_windows(base: u64, len: u64) {
    WINDOWS[0].store(base, Ordering::Relaxed);
    WINDOWS[1].store(len, Ordering::Relaxed);
}

#[cfg_attr(
    feature = "debug-addrs",
    allow(
        dead_code,
        reason = "só o filtro e o `Addr` sem a feature leem as regiões"
    )
)]
fn regions() -> Regions {
    let (image_start, image_end) = vm::image_range();
    let (window, window_len) = (
        WINDOWS[0].load(Ordering::Relaxed),
        WINDOWS[1].load(Ordering::Relaxed),
    );
    Regions::new([
        Region {
            name: "kernel",
            start: image_start,
            len: image_end - image_start,
        },
        Region {
            name: "direct",
            start: DIRECT[0].load(Ordering::Relaxed),
            len: DIRECT[1].load(Ordering::Relaxed),
        },
        Region {
            name: "mmio",
            start: window,
            len: window_len,
        },
        Region {
            name: "stack",
            start: window.saturating_add(window_len),
            len: window_len,
        },
    ])
}

/// Endereço virtual do kernel para `{}`: `kernel+0x1234`, `direct+0x40` ou `<kernel>`.
#[derive(Clone, Copy)]
pub struct Addr(pub u64);

/// Como o `Display`: o `derive` escreveria o número em decimal, que o filtro não reconhece.
impl fmt::Debug for Addr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Addr {
    #[cfg(not(feature = "debug-addrs"))]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&regions().display(self.0), f)
    }

    #[cfg(feature = "debug-addrs")]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

/// Roda `write` sobre `out` trocando todo literal `0x…` de endereço do kernel pela forma
/// de `Addr`. É a rede de segurança para o que o `kprintln!` e o pânico imprimem sem `Addr`.
#[cfg(not(feature = "debug-addrs"))]
pub fn scrubbed<W: Write>(out: W, write: impl FnOnce(&mut dyn Write)) {
    let regions = regions();
    let mut scrubber = Scrubber::new(out, &regions);
    write(&mut scrubber);
}

#[cfg(feature = "debug-addrs")]
pub fn scrubbed<W: Write>(mut out: W, write: impl FnOnce(&mut dyn Write)) {
    write(&mut out);
}
