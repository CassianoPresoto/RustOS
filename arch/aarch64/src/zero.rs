//! Zerar memória Normal por faixas de 64 bytes: `DC ZVA` quando o `DCZID_EL0` deixa, e
//! laço de `STP` quando não. O mapa direto é Normal write-back (SDD 13, SDD 07).

use crate::zero_plan::{LINE, split, zva_block_bytes};
use core::arch::asm;

/// Bloco do `DC ZVA` desta CPU, ou `None` se o `DZP` o proíbe. O `DCZID_EL0` é por CPU e
/// o Linux o lê a cada `clear_page`: ler aqui evita usar o bloco de outro núcleo.
pub fn block_bytes() -> Option<usize> {
    let dczid: u64;
    // SAFETY: `DCZID_EL0` é de leitura e legal em EL1.
    unsafe { asm!("mrs {}, dczid_el0", out(reg) dczid, options(nomem, nostack, preserves_flags)) };
    zva_block_bytes(dczid)
}

/// Zera `[ptr, ptr + len)`; pânico se `ptr` ou `len` não for múltiplo de 64.
///
/// # Safety
/// `ptr` e `len` múltiplos de 64; a faixa é memória Normal gravável, sem outro acesso.
#[allow(clippy::missing_panics_doc, reason = "o pânico está na primeira linha")]
pub unsafe fn zero_aligned(ptr: *mut u8, len: usize) {
    assert!((ptr.expose_provenance() | len).is_multiple_of(LINE));
    let Some(block) = block_bytes() else {
        // SAFETY: o contrato da função.
        return unsafe { store_pairs(ptr, len) };
    };
    let plan = split(ptr.expose_provenance(), len, block);
    let body = ptr.wrapping_add(plan.head);
    let tail = body.wrapping_add(plan.body);
    // SAFETY: a cabeça é a primeira parte de `[ptr, ptr + len)`, pelo contrato de `split`.
    unsafe { store_pairs(ptr, plan.head) };
    // SAFETY: o corpo começa alinhado ao bloco e tem blocos inteiros, também por `split`.
    unsafe { zero_blocks(body, plan.body, block) };
    // SAFETY: a cauda é o fim de `[ptr, ptr + len)` e é múltipla de 64.
    unsafe { store_pairs(tail, plan.tail) };
}

/// # Safety
/// `ptr` e `len` múltiplos de `LINE`, faixa gravável.
unsafe fn store_pairs(ptr: *mut u8, len: usize) {
    assert!(len.is_multiple_of(LINE));
    if len == 0 {
        return;
    }
    // SAFETY: o contrato da função; cada volta grava 64 bytes de `[ptr, ptr + len)`.
    unsafe {
        asm!(
            "2:",
            "stp xzr, xzr, [{p}]",
            "stp xzr, xzr, [{p}, #16]",
            "stp xzr, xzr, [{p}, #32]",
            "stp xzr, xzr, [{p}, #48]",
            "add {p}, {p}, #64",
            "subs {n}, {n}, #64",
            "b.ne 2b",
            p = inout(reg) ptr => _, n = inout(reg) len => _,
            options(nostack),
        );
    }
}

/// # Safety
/// `ptr` alinhado a `block`, `len` múltiplo de `block`, `block` o do `DCZID_EL0`, memória Normal.
unsafe fn zero_blocks(ptr: *mut u8, len: usize, block: usize) {
    if len == 0 {
        return;
    }
    // SAFETY: o contrato da função; cada `dc zva` zera um bloco inteiro dentro da faixa.
    unsafe {
        asm!(
            "2:",
            "dc zva, {p}",
            "add {p}, {p}, {b}",
            "subs {n}, {n}, {b}",
            "b.ne 2b",
            p = inout(reg) ptr => _, n = inout(reg) len => _, b = in(reg) block,
            options(nostack),
        );
    }
}
