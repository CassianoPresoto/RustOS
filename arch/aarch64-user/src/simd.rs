//! O teste de `v0` entre processos (SDD 17, "SIMD de usuário"): põe um valor na lane baixa, confere
//! o registrador em laço e o restaura. Todo o `asm!` do teste mora aqui, num bloco só.

use core::arch::asm;

/// Põe `value` na lane baixa de `v0`, confere `rounds` vezes que ela se manteve e devolve `v0` como
/// estava. Devolve `false` na primeira diferença.
pub fn simd_hold(value: u64, rounds: u64) -> bool {
    // O registrador inteiro vai para a memória: `fmov` para `d0` zera a lane alta.
    let mut saved = [0_u128; 1];
    let held: u64;
    // SAFETY: `saved` tem 16 bytes e vive até o fim do bloco. Só GPRs e flags são clobber, e todo
    // caminho de saída restaura `v0`; `sp` não muda (`nostack`).
    unsafe {
        asm!(
            ".arch_extension fp", ".arch_extension simd",
            "str q0, [{buf}]",
            "fmov d0, {value}",
            "mov {ok}, #1",
            "cbz {rounds}, 5f",
            "2:",
            "fmov {got}, d0",
            "cmp {got}, {value}",
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
            rounds = inout(reg) rounds => _,
            got = out(reg) _,
            ok = out(reg) held,
            options(nostack),
        );
    }
    held != 0
}
