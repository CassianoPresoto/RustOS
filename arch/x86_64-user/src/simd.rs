//! O teste de `xmm0` entre processos (SDD 17, "SIMD de usuário"): põe um valor na lane baixa, confere
//! o registrador em laço e o restaura. Todo o `asm!` do teste mora aqui, num bloco só.

use core::arch::asm;

/// Põe `value` na lane baixa de `xmm0`, confere `rounds` vezes que ela se manteve e devolve `xmm0` como
/// estava. Devolve `false` na primeira diferença.
pub fn simd_hold(value: u64, rounds: u64) -> bool {
    // O registrador inteiro vai para a memória: `movq` de GPR zera a lane alta.
    let mut saved = [0_u128; 1];
    let held: u64;
    // SAFETY: `saved` tem 16 bytes e vive até o fim do bloco. Só GPRs e flags são clobber, e todo
    // caminho de saída restaura `xmm0`; `rsp` não muda (`nostack`).
    unsafe {
        asm!(
            "movdqu [{buf}], xmm0",
            "movq xmm0, {value}",
            "mov {ok}, 1",
            "test {rounds}, {rounds}",
            "jz 5f",
            "2:",
            "movq {got}, xmm0",
            "cmp {got}, {value}",
            "jne 4f",
            "pause",
            "dec {rounds}",
            "jnz 2b",
            "jmp 5f",
            "4:",
            "mov {ok}, 0",
            "5:",
            "movdqu xmm0, [{buf}]",
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
