//! Estado estendido da CPU (x87, SSE, AVX, AVX-512): que componentes ligar em
//! `XCR0`, o tamanho da área de salvamento e o estado inicial dela. Puro.
//! Bits de `XCR0` em `arch/x86/include/asm/fpu/types.h` do Linux; MXCSR e FCW
//! iniciais em `fpstate_init_fxstate` e `MXCSR_DEFAULT` do mesmo diretório.

pub const XCR0_X87: u64 = 1 << 0;
pub const XCR0_SSE: u64 = 1 << 1;
pub const XCR0_AVX: u64 = 1 << 2;
/// Os três componentes do AVX-512: máscaras `k0`-`k7`, ZMM0-15 (bits 256-511) e ZMM16-31.
pub const XCR0_AVX512: u64 = 0b111 << 5;

/// Área legada do FXSAVE (512 bytes) mais o cabeçalho do XSAVE (64): o mínimo.
pub const MIN_XSAVE_BYTES: usize = 576;
/// O XSAVE exige a área alinhada a 64 bytes (SDM, instrução XSAVE).
pub const AREA_ALIGN: usize = 64;

const FCW_OFFSET: usize = 0;
const MXCSR_OFFSET: usize = 24;
const FCW_DEFAULT: u16 = 0x037F;
const MXCSR_DEFAULT: u32 = 0x1F80;

/// Como a troca de contexto guarda o estado, do mais ao menos capaz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveMode {
    /// `XSAVEOPT`: pula o componente que não mudou desde o último `XRSTOR`.
    XsaveOpt,
    Xsave,
    /// Só x87 e SSE, 512 bytes.
    Fxsave,
}

impl SaveMode {
    pub const fn name(self) -> &'static str {
        match self {
            Self::XsaveOpt => "xsaveopt",
            Self::Xsave => "xsave",
            Self::Fxsave => "fxsave",
        }
    }
}

pub const fn save_mode(xsave: bool, xsaveopt: bool) -> SaveMode {
    match (xsave, xsaveopt) {
        (true, true) => SaveMode::XsaveOpt,
        (true, false) => SaveMode::Xsave,
        (false, _) => SaveMode::Fxsave,
    }
}

/// O `XCR0` a escrever, dado o que a CPU suporta (`CPUID.0DH.0:EDX:EAX`): x87 e
/// SSE sempre; AVX se houver; AVX-512 só com os três componentes juntos.
pub const fn wanted_xcr0(supported: u64) -> u64 {
    let mut xcr0 = XCR0_X87 | XCR0_SSE;
    if supported & XCR0_AVX != 0 {
        xcr0 |= XCR0_AVX;
        if supported & XCR0_AVX512 == XCR0_AVX512 {
            xcr0 |= XCR0_AVX512;
        }
    }
    xcr0
}

/// Bytes da área para o `XCR0` ligado (`CPUID.0DH.0:EBX`), no mínimo o legado
/// mais o cabeçalho e arredondado a 64.
pub const fn area_bytes(cpuid_ebx: u32) -> usize {
    let bytes = if (cpuid_ebx as usize) < MIN_XSAVE_BYTES {
        MIN_XSAVE_BYTES
    } else {
        cpuid_ebx as usize
    };
    bytes.next_multiple_of(AREA_ALIGN)
}

/// Os componentes ligados, por nome, para o relatório do boot.
pub const fn components(xcr0: u64) -> &'static str {
    if xcr0 & XCR0_AVX512 == XCR0_AVX512 {
        "x87 sse avx avx512"
    } else if xcr0 & XCR0_AVX != 0 {
        "x87 sse avx"
    } else {
        "x87 sse"
    }
}

/// Escreve o estado inicial numa área zerada: `XSTATE_BV` zero manda o `XRSTOR`
/// iniciar os componentes, e FCW e MXCSR ficam nos valores de reset.
pub fn init_area(area: &mut [u8]) {
    area.fill(0);
    area[FCW_OFFSET..FCW_OFFSET + 2].copy_from_slice(&FCW_DEFAULT.to_le_bytes());
    area[MXCSR_OFFSET..MXCSR_OFFSET + 4].copy_from_slice(&MXCSR_DEFAULT.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_enables_x87_and_sse() {
        assert_eq!(wanted_xcr0(0b11), XCR0_X87 | XCR0_SSE);
        assert_eq!(wanted_xcr0(0), XCR0_X87 | XCR0_SSE);
    }

    #[test]
    fn enables_avx_only_when_supported() {
        assert_eq!(wanted_xcr0(0b111), 0b111);
        assert_eq!(wanted_xcr0(1 << 5), 0b11);
    }

    #[test]
    fn avx512_needs_all_three_components() {
        let partial = 0b111 | (0b011 << 5);
        assert_eq!(wanted_xcr0(partial), 0b111);
        let full = 0b111 | XCR0_AVX512 | (1 << 9);
        assert_eq!(wanted_xcr0(full), 0b111 | XCR0_AVX512);
    }

    #[test]
    fn avx512_without_avx_is_not_enabled() {
        assert_eq!(wanted_xcr0(0b11 | XCR0_AVX512), 0b11);
    }

    #[test]
    fn area_is_at_least_legacy_plus_header_and_64_aligned() {
        assert_eq!(area_bytes(0), 576);
        assert_eq!(area_bytes(512), 576);
        assert_eq!(area_bytes(832), 832);
        assert_eq!(area_bytes(833), 896);
        assert_eq!(area_bytes(2696), 2752);
    }

    #[test]
    fn save_mode_prefers_xsaveopt_and_falls_back() {
        assert_eq!(save_mode(true, true), SaveMode::XsaveOpt);
        assert_eq!(save_mode(true, false), SaveMode::Xsave);
        assert_eq!(save_mode(false, true), SaveMode::Fxsave);
        assert_eq!(save_mode(false, false), SaveMode::Fxsave);
    }

    #[test]
    fn initial_area_has_reset_fcw_mxcsr_and_empty_state_bv() {
        let mut area = [0xAA_u8; 576];
        init_area(&mut area);
        assert_eq!(u16::from_le_bytes([area[0], area[1]]), 0x037F);
        assert_eq!(
            u32::from_le_bytes([area[24], area[25], area[26], area[27]]),
            0x1F80
        );
        assert!(area[512..576].iter().all(|&byte| byte == 0));
        assert!(area[32..512].iter().all(|&byte| byte == 0));
    }

    #[test]
    fn names_follow_the_enabled_components() {
        assert_eq!(components(0b11), "x87 sse");
        assert_eq!(components(0b111), "x87 sse avx");
        assert_eq!(components(0b111 | XCR0_AVX512), "x87 sse avx avx512");
    }
}
