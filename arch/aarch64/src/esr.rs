//! Decodificação pura do `ESR_EL1` e do `FAR_EL1` em `hal::FaultKind`.
//! Classes e códigos de falha conferidos contra `arch/arm64/include/asm/esr.h` do Linux.

use rustos_hal::{FaultKind, VirtAddr};

const EC_SHIFT: u32 = 26;
const EC_UNKNOWN: u64 = 0x00;
const EC_IABT_LOW: u64 = 0x20;
const EC_IABT_CUR: u64 = 0x21;
const EC_PC_ALIGN: u64 = 0x22;
const EC_DABT_LOW: u64 = 0x24;
const EC_DABT_CUR: u64 = 0x25;
const EC_SP_ALIGN: u64 = 0x26;
const EC_SERROR: u64 = 0x2f;
const EC_BRK64: u64 = 0x3c;
/// `WnR`: o acesso que falhou foi uma escrita (só vale em aborto de dado).
const ISS_WNR: u64 = 1 << 6;
const FSC_MASK: u64 = 0x3f;
const FSC_ALIGNMENT: u8 = 0b10_0001;
/// Falta de tradução no nível −1 (só com `FEAT_LPA2`).
const FSC_TRANSLATION_MINUS_1: u8 = 0b10_1011;

/// Por que a MMU recusou o acesso (`DFSC` em aborto de dado, `IFSC` em instrução).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultStatus {
    /// Descritor inválido no nível dado (−1 a 3): a página não está mapeada.
    Translation(i8),
    /// Descritor válido sem o bit `AF`.
    AccessFlag(u8),
    /// Descritor válido que não deixa este acesso.
    Permission(u8),
    Alignment,
    /// Aborto externo, paridade, conflito de TLB… O código cru do `FSC`.
    Other(u8),
}

pub const fn exception_class(esr: u64) -> u64 {
    esr >> EC_SHIFT & 0x3f
}

/// Decodifica o campo `FSC` (6 bits) do `ISS` de um aborto de dado ou de instrução.
pub const fn fault_status(esr: u64) -> FaultStatus {
    #[allow(clippy::cast_possible_truncation, reason = "a máscara deixa 6 bits")]
    let fsc = (esr & FSC_MASK) as u8;
    let level = (fsc & 0b11).cast_signed();
    match fsc >> 2 {
        0b0001 => FaultStatus::Translation(level),
        0b0010 => FaultStatus::AccessFlag(fsc & 0b11),
        0b0011 => FaultStatus::Permission(fsc & 0b11),
        _ if fsc == FSC_TRANSLATION_MINUS_1 => FaultStatus::Translation(-1),
        _ if fsc == FSC_ALIGNMENT => FaultStatus::Alignment,
        _ => FaultStatus::Other(fsc),
    }
}

/// A falha foi uma escrita (`WnR`). Em aborto de instrução o bit não existe: `false`.
pub const fn is_write(esr: u64) -> bool {
    matches!(exception_class(esr), EC_DABT_LOW | EC_DABT_CUR) && esr & ISS_WNR != 0
}

/// Traduz `ESR_EL1` e `FAR_EL1` na falha que o núcleo entende. `far` só é lido
/// nos abortos de tradução, acesso e permissão, onde o `FnV` é zero por definição.
pub const fn decode(esr: u64, far: u64) -> FaultKind {
    let class = exception_class(esr);
    match class {
        EC_IABT_LOW | EC_IABT_CUR | EC_DABT_LOW | EC_DABT_CUR => {
            let status = fault_status(esr);
            match status {
                FaultStatus::Alignment => FaultKind::Alignment,
                FaultStatus::Other(_) => FaultKind::Other,
                FaultStatus::Translation(_)
                | FaultStatus::AccessFlag(_)
                | FaultStatus::Permission(_) => FaultKind::PageFault {
                    address: VirtAddr::new(far),
                    write: is_write(esr),
                    execute: matches!(class, EC_IABT_LOW | EC_IABT_CUR),
                    user: matches!(class, EC_IABT_LOW | EC_DABT_LOW),
                    present: !matches!(status, FaultStatus::Translation(_)),
                },
            }
        }
        EC_BRK64 => FaultKind::Breakpoint,
        EC_UNKNOWN => FaultKind::InvalidOpcode,
        EC_PC_ALIGN | EC_SP_ALIGN => FaultKind::Alignment,
        EC_SERROR => FaultKind::MachineCheck,
        _ => FaultKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn esr(class: u64, iss: u64) -> u64 {
        class << EC_SHIFT | 1 << 25 | iss
    }

    fn page_fault(kind: FaultKind) -> (u64, bool, bool, bool, bool) {
        match kind {
            FaultKind::PageFault {
                address,
                write,
                execute,
                user,
                present,
            } => (address.as_u64(), write, execute, user, present),
            other => panic!("esperava page fault, veio {other:?}"),
        }
    }

    #[test]
    fn kernel_read_of_an_unmapped_page_is_a_not_present_page_fault() {
        let kind = decode(esr(EC_DABT_CUR, 0b00_0111), 0xffff_fe80_0000_0000);
        assert_eq!(
            page_fault(kind),
            (0xffff_fe80_0000_0000, false, false, false, false)
        );
    }

    #[test]
    fn wnr_makes_a_data_abort_a_write() {
        let kind = decode(esr(EC_DABT_CUR, ISS_WNR | 0b00_1101), 0x1000);
        assert_eq!(page_fault(kind), (0x1000, true, false, false, true));
    }

    #[test]
    fn el0_aborts_are_user_faults() {
        let data = decode(esr(EC_DABT_LOW, 0b00_0100), 0x10);
        assert_eq!(page_fault(data), (0x10, false, false, true, false));
        let fetch = decode(esr(EC_IABT_LOW, 0b00_0100), 0x20);
        assert_eq!(page_fault(fetch), (0x20, false, true, true, false));
    }

    #[test]
    fn instruction_abort_is_an_execute_fault_and_never_a_write() {
        let kind = decode(esr(EC_IABT_CUR, ISS_WNR | 0b00_1111), 0x4000);
        assert_eq!(page_fault(kind), (0x4000, false, true, false, true));
    }

    #[test]
    fn translation_levels_follow_the_fsc_encoding() {
        for level in 0..4 {
            let status = fault_status(esr(EC_DABT_CUR, 0b00_0100 | level));
            assert_eq!(
                status,
                FaultStatus::Translation(i8::try_from(level).unwrap())
            );
        }
        assert_eq!(fault_status(0b10_1011), FaultStatus::Translation(-1));
    }

    #[test]
    fn access_flag_and_permission_faults_report_their_level() {
        assert_eq!(fault_status(0b00_1010), FaultStatus::AccessFlag(2));
        assert_eq!(fault_status(0b00_1111), FaultStatus::Permission(3));
        assert_eq!(fault_status(0b00_1100), FaultStatus::Permission(0));
    }

    #[test]
    fn access_flag_fault_counts_as_present() {
        let (.., present) = page_fault(decode(esr(EC_DABT_CUR, 0b00_1011), 0));
        assert!(present);
    }

    #[test]
    fn data_abort_alignment_fault_is_an_alignment_fault() {
        assert_eq!(
            decode(esr(EC_DABT_CUR, FSC_ALIGNMENT.into()), 0),
            FaultKind::Alignment
        );
    }

    #[test]
    fn external_aborts_are_not_page_faults() {
        assert_eq!(decode(esr(EC_DABT_CUR, 0b01_0000), 0), FaultKind::Other);
        assert_eq!(decode(esr(EC_IABT_CUR, 0b11_0000), 0), FaultKind::Other);
        assert_eq!(fault_status(0b01_0100), FaultStatus::Other(0b01_0100));
    }

    #[test]
    fn brk_is_a_breakpoint_whatever_the_immediate() {
        assert_eq!(decode(esr(EC_BRK64, 0), 0), FaultKind::Breakpoint);
        assert_eq!(decode(esr(EC_BRK64, 0xffff), 0), FaultKind::Breakpoint);
    }

    #[test]
    fn pc_and_sp_misalignment_are_alignment_faults() {
        assert_eq!(decode(esr(EC_PC_ALIGN, 0), 0), FaultKind::Alignment);
        assert_eq!(decode(esr(EC_SP_ALIGN, 0), 0), FaultKind::Alignment);
    }

    #[test]
    fn unknown_class_is_an_invalid_opcode_and_serror_a_machine_check() {
        assert_eq!(decode(esr(EC_UNKNOWN, 0), 0), FaultKind::InvalidOpcode);
        assert_eq!(decode(esr(EC_SERROR, 0), 0), FaultKind::MachineCheck);
    }

    #[test]
    fn other_classes_keep_their_class_in_the_raw_code_only() {
        assert_eq!(decode(esr(0x15, 0), 0), FaultKind::Other);
        assert_eq!(exception_class(esr(0x15, 0x1234)), 0x15);
    }

    #[test]
    fn wnr_is_ignored_outside_data_aborts() {
        assert!(!is_write(esr(EC_IABT_CUR, ISS_WNR)));
        assert!(is_write(esr(EC_DABT_LOW, ISS_WNR)));
    }
}
