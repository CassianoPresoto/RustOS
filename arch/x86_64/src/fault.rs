//! Vetores da IDT e decodificação de exceção em `hal::Fault`. Puro.

use rustos_hal::{Fault, FaultKind, VirtAddr};

pub const DIVIDE_ERROR: u8 = 0;
pub const NMI: u8 = 2;
pub const DEBUG: u8 = 1;
pub const BREAKPOINT: u8 = 3;
pub const INVALID_OPCODE: u8 = 6;
pub const DOUBLE_FAULT: u8 = 8;
pub const GENERAL_PROTECTION: u8 = 13;
pub const PAGE_FAULT: u8 = 14;
pub const ALIGNMENT_CHECK: u8 = 17;
pub const MACHINE_CHECK: u8 = 18;
/// As exceções ocupam 0 a 31; daí em diante o vetor é de IRQ.
pub const FIRST_IRQ_VECTOR: u8 = 32;

/// Vetores em que a CPU empilha código de erro (`exception_has_error_code` da QEMU e
/// `idtentry.h` do Linux; a SDM não foi lida): #DF, #TS, #NP, #SS, #GP, #PF, #AC,
/// #CP e, na AMD, #VC e #SX.
pub const ERROR_CODE_VECTORS: u64 = 1 << 8
    | 1 << 10
    | 1 << 11
    | 1 << 12
    | 1 << 13
    | 1 << 14
    | 1 << 17
    | 1 << 21
    | 1 << 29
    | 1 << 30;

pub const fn has_error_code(vector: u8) -> bool {
    vector < FIRST_IRQ_VECTOR && ERROR_CODE_VECTORS >> vector & 1 != 0
}

/// Bits do código de erro do #PF (SDM vol. 3A, seção 4.7).
const PF_PRESENT: u64 = 1 << 0;
pub const PF_WRITE: u64 = 1 << 1;
const PF_USER: u64 = 1 << 2;
const PF_INSTRUCTION: u64 = 1 << 4;

/// `vector << 32 | código de erro`, como `Fault::code` documenta.
pub const fn raw_code(vector: u8, error: u64) -> u64 {
    (vector as u64) << 32 | error & 0xFFFF_FFFF
}

pub const fn decode(vector: u8, error: u64, rip: u64, cr2: u64) -> Fault {
    let kind = match vector {
        DIVIDE_ERROR => FaultKind::DivideError,
        BREAKPOINT => FaultKind::Breakpoint,
        INVALID_OPCODE => FaultKind::InvalidOpcode,
        DOUBLE_FAULT => FaultKind::DoubleFault,
        GENERAL_PROTECTION => FaultKind::GeneralProtection,
        ALIGNMENT_CHECK => FaultKind::Alignment,
        MACHINE_CHECK => FaultKind::MachineCheck,
        PAGE_FAULT => FaultKind::PageFault {
            address: VirtAddr::new(cr2),
            write: error & PF_WRITE != 0,
            execute: error & PF_INSTRUCTION != 0,
            user: error & PF_USER != 0,
            present: error & PF_PRESENT != 0,
        },
        _ => FaultKind::Other,
    };
    Fault {
        kind,
        pc: VirtAddr::new(rip),
        code: raw_code(vector, error),
        from_user: false,
        user_sp: 0,
    }
}

/// A exceção que veio do anel 3 mata só o processo, salvo NMI, `#DF` e `#MC`: fatais com
/// relatório mesmo vindas de usuário (SDD 17, "Exceções vindas do anel 3").
pub const fn user_fault_kills_only_the_process(vector: u8) -> bool {
    !matches!(vector, NMI | DOUBLE_FAULT | MACHINE_CHECK)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_vectors_match_the_manual() {
        let with: Vec<u8> = (0..=255).filter(|&v| has_error_code(v)).collect();
        assert_eq!(with, [8, 10, 11, 12, 13, 14, 17, 21, 29, 30]);
    }

    #[test]
    fn irq_vectors_never_carry_an_error_code() {
        assert!(!has_error_code(0x20));
        assert!(!has_error_code(0xFF));
    }

    #[test]
    fn page_fault_bits_become_fields() {
        let fault = decode(PAGE_FAULT, 0b10011, 0xffff_ffff_8000_1000, 0xdead_b000);
        assert_eq!(
            fault.kind,
            FaultKind::PageFault {
                address: VirtAddr::new(0xdead_b000),
                write: true,
                execute: true,
                user: false,
                present: true,
            }
        );
        assert_eq!(fault.pc.as_u64(), 0xffff_ffff_8000_1000);
        assert_eq!(fault.code, 14 << 32 | 0b10011);
    }

    #[test]
    fn not_present_user_read_is_decoded() {
        let fault = decode(PAGE_FAULT, 0b100, 0, 0x10);
        assert_eq!(
            fault.kind,
            FaultKind::PageFault {
                address: VirtAddr::new(0x10),
                write: false,
                execute: false,
                user: true,
                present: false,
            }
        );
    }

    #[test]
    fn vectors_map_to_their_kinds() {
        assert_eq!(decode(3, 0, 0, 0).kind, FaultKind::Breakpoint);
        assert_eq!(decode(0, 0, 0, 0).kind, FaultKind::DivideError);
        assert_eq!(decode(6, 0, 0, 0).kind, FaultKind::InvalidOpcode);
        assert_eq!(decode(8, 0, 0, 0).kind, FaultKind::DoubleFault);
        assert_eq!(decode(13, 0, 0, 0).kind, FaultKind::GeneralProtection);
        assert_eq!(decode(17, 0, 0, 0).kind, FaultKind::Alignment);
        assert_eq!(decode(18, 0, 0, 0).kind, FaultKind::MachineCheck);
        assert_eq!(decode(2, 0, 0, 0).kind, FaultKind::Other);
        assert_eq!(decode(21, 0, 0, 0).kind, FaultKind::Other);
    }

    #[test]
    fn only_the_cpu_wide_faults_stay_fatal_when_they_come_from_user() {
        for vector in [NMI, DOUBLE_FAULT, MACHINE_CHECK] {
            assert!(!user_fault_kills_only_the_process(vector));
        }
        for vector in [0, DEBUG, 3, 6, 7, 13, 14, 17, 19] {
            assert!(user_fault_kills_only_the_process(vector));
        }
    }

    #[test]
    fn decoded_faults_start_as_kernel_origin() {
        let fault = decode(13, 0, 0x1000, 0);
        assert!(!fault.from_user);
        assert_eq!(fault.user_sp, 0);
    }

    #[test]
    fn raw_code_keeps_only_the_low_32_bits_of_the_error() {
        assert_eq!(raw_code(13, 0x1_0000_0010), 13 << 32 | 0x10);
    }
}
