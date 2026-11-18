//! A máscara do clamp de índice por `cmp` e `sbb`, como o `array_index_mask_nospec` do Linux
//! (`arch/x86/include/asm/barrier.h`, v6.12): o compilador não pode trocá-la por desvio.

use core::arch::asm;

/// Todo bit ligado se `index < size` (sem sinal), zero caso contrário.
#[inline]
pub fn array_index_mask_nospec(index: usize, size: usize) -> usize {
    let mask: usize;
    // SAFETY: só aritmética sobre registradores; o `cmp` deixa o vai-um em `CF` e o `sbb` o expande.
    unsafe {
        asm!(
            "cmp {index}, {size}",
            "sbb {mask}, {mask}",
            index = in(reg) index,
            size = in(reg) size,
            mask = out(reg) mask,
            options(pure, nomem, nostack),
        );
    }
    mask
}
