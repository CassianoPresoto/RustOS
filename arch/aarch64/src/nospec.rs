//! A barreira de dados do clamp de índice (`CSDB`), que o Linux põe depois da máscara.

use core::arch::asm;

/// Executa `CSDB` (`HINT #20`) entre o cálculo da máscara e o seu uso; `mask` passa pela
/// instrução para o compilador não mover nem a conta nem o `AND` para o outro lado dela.
#[inline]
pub fn csdb(mask: usize) -> usize {
    let mut mask = mask;
    // SAFETY: `hint #20` é um `nop` onde o `CSDB` não existe e não toca memória nem registrador.
    unsafe {
        asm!("hint #20 /* {0} */", inout(reg) mask, options(nomem, nostack, preserves_flags));
    }
    mask
}
