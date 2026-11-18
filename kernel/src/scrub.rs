//! Limpeza da pilha do kernel na volta ao usuário (SDD 17; paga DEBT-0059). A pilha da tarefa de
//! usuário é pintada com `STACK_PAINT`; na volta, o que a chamada usou abaixo do quadro volta a
//! ser pintura, e o `arch/` repinta os 512 bytes que ficam logo abaixo dele.

use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rustos_hal::{PAGE_SIZE, STACK_PAINT};
use rustos_mm::watermark::lowest_dirty_below;

use crate::task::STACK_PAGES;

/// Palavras limpas seguidas que encerram a varredura: 128 bytes. Um quadro com mais que isso de
/// locais sem escrever esconderia o que está abaixo dele; as chamadas não têm um assim.
const CLEAN_RUN_WORDS: usize = 16;
/// O que o `arch/` repinta sem usar pilha, abaixo do quadro (igual ao do `asm`).
const ARCH_WINDOW_BYTES: usize = 256;
const WORD: usize = 8;
#[allow(
    clippy::cast_possible_truncation,
    reason = "o kernel só existe em 64 bits"
)]
const PAGE: usize = PAGE_SIZE as usize;
#[allow(
    clippy::cast_possible_truncation,
    reason = "o kernel só existe em 64 bits"
)]
const STACK_BYTES: usize = STACK_PAGES as usize * PAGE;

/// Liga e desliga a limpeza; só o modo de teste a desliga, para medir o custo.
static ENABLED: AtomicBool = AtomicBool::new(true);
/// As medidas de profundidade valem: o autoteste as desliga nas rodadas que planta resíduo.
static MEASURING: AtomicBool = AtomicBool::new(true);
/// O maior fundo que a pilha de uma tarefa de usuário alcançou na volta ao usuário, em bytes.
static DEEPEST: AtomicUsize = AtomicUsize::new(0);

#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "só o modo de teste desliga a limpeza")
)]
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
    crate::platform::set_stack_clean(on);
}

#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "só o modo de teste desliga as medidas")
)]
pub fn set_measuring(on: bool) {
    MEASURING.store(on, Ordering::Relaxed);
}

pub fn measuring() -> bool {
    MEASURING.load(Ordering::Relaxed)
}

#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "lido só pelo modo de teste")
)]
pub fn deepest() -> usize {
    DEEPEST.load(Ordering::Relaxed)
}

fn word(address: usize) -> *mut u64 {
    ptr::with_exposed_provenance_mut(address)
}

/// O fim da pilha que contém `frame`: pilhas das threads terminam em página, e o quadro de
/// usuário fica no topo, a menos de uma página dele.
fn top_of(frame: usize) -> usize {
    (frame + PAGE) & !(PAGE - 1)
}

/// `Handlers::leave_user`: `frame` é o quadro que o `arch/` salvou no topo da pilha da tarefa.
#[inline(never)]
pub fn leave_user(frame: usize) {
    let top = top_of(frame);
    let bottom = top - STACK_BYTES;
    let window = ARCH_WINDOW_BYTES / WORD;
    let from = (frame - bottom) / WORD - window;
    let lowest = lowest_dirty_below(from, STACK_PAINT, CLEAN_RUN_WORDS, |index| {
        // SAFETY: `[bottom, top)` é a pilha desta tarefa, mapeada para sempre; só se lê abaixo
        // do quadro, onde nenhum quadro vivo desta CPU escreve.
        unsafe { word(bottom + index * WORD).read_volatile() }
    });
    let depth = top - (bottom + lowest * WORD);
    if depth > DEEPEST.load(Ordering::Relaxed) && MEASURING.load(Ordering::Relaxed) {
        DEEPEST.fetch_max(depth, Ordering::Relaxed);
        crate::task::note_user_depth(depth);
    }
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    for index in lowest..from {
        // SAFETY: idem; fica abaixo da janela que a função e o `arch/` ocupam.
        unsafe { word(bottom + index * WORD).write_volatile(STACK_PAINT) };
    }
}
