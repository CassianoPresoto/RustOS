//! A máscara do clamp de índice por `cmp`, `sbc` e `CSDB`, a sequência do
//! `array_index_mask_nospec` do Linux (`arch/arm64/include/asm/barrier.h`, v6.12).

use core::arch::asm;

/// Todo bit ligado se `index < size` (sem sinal), zero caso contrário; a `CSDB` fecha a máscara
/// antes de qualquer uso dela (DEBT-0105).
#[inline]
pub fn array_index_mask_nospec(index: usize, size: usize) -> usize {
    let mask: usize;
    // SAFETY: só aritmética e `hint #20` (`nop` onde a `CSDB` não existe); sem memória, sem pilha.
    unsafe {
        asm!(
            "cmp {index}, {size}",
            "sbc {mask}, xzr, xzr",
            "hint #20",
            index = in(reg) index,
            size = in(reg) size,
            mask = out(reg) mask,
            options(nomem, nostack),
        );
    }
    mask
}
