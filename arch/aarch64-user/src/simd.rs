//! O teste de `v0` entre processos (SDD 17, "SIMD de usuário"): põe um valor na lane baixa e o NOT
//! dele na alta, confere as duas em laço e restaura o registrador. Todo o `asm!` do teste mora aqui.

use core::arch::asm;

/// Põe `value` na lane baixa de `v0` e `!value` na alta, confere `rounds` vezes que as duas se
/// mantiveram e devolve `v0` como estava. Devolve `false` na primeira diferença.
pub fn simd_hold(value: u64, rounds: u64) -> bool {
    let mut saved = [0_u128; 1];
    let inverse = !value;
    let held: u64;
    // SAFETY: `saved` tem 16 bytes e vive até o fim do bloco. Só GPRs e flags são clobber; todo
    // caminho de saída restaura `v0`, e `sp` não muda (`nostack`). `fmov` zera a alta, então ela é
    // posta de novo com `mov v0.d[1]`.
    unsafe {
        asm!(
            ".arch_extension fp", ".arch_extension simd",
            "str q0, [{buf}]",
            "fmov d0, {value}",
            "mov v0.d[1], {inv}",
            "mov {ok}, #1",
            "cbz {rounds}, 5f",
            "2:",
            "fmov {got}, d0",
            "cmp {got}, {value}",
            "b.ne 4f",
            "mov {got}, v0.d[1]",
            "cmp {got}, {inv}",
            "b.ne 4f",
            "yield",
            "subs {rounds}, {rounds}, #1",
            "b.ne 2b",
            "b 5f",
            "4:",
            "mov {ok}, #0",
            "5:",
            "ldr q0, [{buf}]",
            buf = in(reg) saved.as_mut_ptr(),
            value = in(reg) value,
            inv = in(reg) inverse,
            rounds = inout(reg) rounds => _,
            got = out(reg) _,
            ok = out(reg) held,
            options(nostack),
        );
    }
    held != 0
}
