//! Manutenção de cache para código que o kernel escreveu e o usuário vai executar: o `CTR_EL0` do
//! HVF é `IDC=1`, `DIC=0`, então a instrução precisa de `ic ivau` mesmo sem limpar o dado.

use core::arch::asm;

/// `CTR_EL0.DminLine` (bits 19:16) e `IminLine` (bits 3:0): log2 do tamanho da linha em palavras.
const fn line_bytes(ctr: u64, shift: u32) -> usize {
    4 << (ctr >> shift & 0xF)
}

/// Tamanho das linhas de dado e de instrução, em bytes, do `CTR_EL0` (puro, testável).
pub const fn line_sizes(ctr: u64) -> (usize, usize) {
    (line_bytes(ctr, 16), line_bytes(ctr, 0))
}

/// Deixa `[start, start + len)` (endereço do kernel) visível como instrução em todas as CPUs.
/// # Safety
/// EL1; a faixa está mapeada e legível.
pub unsafe fn sync_icache(start: usize, len: usize) {
    let ctr: u64;
    // SAFETY: leitura de registrador de identificação, sem efeito colateral.
    unsafe { asm!("mrs {}, ctr_el0", out(reg) ctr, options(nomem, nostack, preserves_flags)) };
    let (dline, iline) = line_sizes(ctr);
    let end = start + len;
    let mut at = start & !(dline - 1);
    while at < end {
        // SAFETY: limpar uma linha de dado por endereço mapeado não tem efeito além do cache.
        unsafe { asm!("dc cvau, {}", in(reg) at, options(nostack, preserves_flags)) };
        at += dline;
    }
    // SAFETY: barreira sem pré-condição.
    unsafe { asm!("dsb ish", options(nostack, preserves_flags)) };
    at = start & !(iline - 1);
    while at < end {
        // SAFETY: invalidar a instrução por endereço mapeado só descarta cache.
        unsafe { asm!("ic ivau, {}", in(reg) at, options(nostack, preserves_flags)) };
        at += iline;
    }
    // SAFETY: as barreiras publicam a invalidação a todas as CPUs e a este fluxo.
    unsafe { asm!("dsb ish", "isb", options(nostack, preserves_flags)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_sizes_come_from_the_ctr_fields() {
        assert_eq!(line_sizes(0x8444_C004), (64, 64));
        assert_eq!(line_sizes(0x0003_0003), (32, 32));
        assert_eq!(line_sizes(0x0004_0000), (64, 4));
    }
}
