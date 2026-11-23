//! O teste de `xmm0` entre processos (SDD 17, "SIMD de usuário"): põe um valor na lane baixa e o NOT
//! dele na alta, confere as duas em laço e restaura o registrador. Todo o `asm!` do teste mora aqui.

use core::arch::asm;

/// Põe `value` na lane baixa de `xmm0` e `!value` na alta, confere `rounds` vezes que as duas se
/// mantiveram e devolve `xmm0` como estava. Devolve `false` na primeira diferença.
pub fn simd_hold(value: u64, rounds: u64) -> bool {
    // [0]: o `xmm0` original. [1]: o rascunho em que cada volta grava `xmm0` para conferir.
    let mut saved = [0_u128; 2];
    let inverse = !value;
    let held: u64;
    // SAFETY: `saved` tem 32 bytes e vive até o fim do bloco. Só GPRs e flags são clobber; todo
    // caminho de saída restaura `xmm0`, e `rsp` não muda (`nostack`). SSE2 basta: sem `pinsrq`.
    unsafe {
        asm!(
            "movdqu [{buf}], xmm0",
            "mov qword ptr [{buf} + 16], {inv}",
            "movq xmm0, {value}",
            "movhps xmm0, qword ptr [{buf} + 16]",
            "mov {ok}, 1",
            "test {rounds}, {rounds}",
            "jz 5f",
            "2:",
            "movdqu [{buf} + 16], xmm0",
            "cmp qword ptr [{buf} + 16], {value}",
            "jne 4f",
            "cmp qword ptr [{buf} + 24], {inv}",
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
            inv = in(reg) inverse,
            rounds = inout(reg) rounds => _,
            ok = out(reg) held,
            options(nostack),
        );
    }
    held != 0
}
