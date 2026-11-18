//! Valores dos MSRs de `SYSCALL` e a conferência de saída por `SYSRET`. Puro: o `asm!` que usa
//! isto mora em `traps` (a saída de `rustos_isr_common_swapped`), que repete as mesmas condições
//! em assembly: mudar uma pede mudar a outra (SDD 17, "x86_64: SYSCALL/SYSRET").

use rustos_hal::abi::USER_TOP;

use crate::segments::{KERNEL_CODE, USER_CODE, USER_DATA};

/// `IA32_STAR`: `SYSRET` soma 16 e 8 ao campo de cima (`0x10` dá `CS = 0x20|3` e `SS = 0x18|3`);
/// `SYSCALL` carrega `CS = 0x08` e `SS = 0x10` do campo do meio.
pub const STAR: u64 = 0x10 << 48 | (KERNEL_CODE as u64) << 32;

const CF: u64 = 1 << 0;
const PF: u64 = 1 << 2;
const AF: u64 = 1 << 4;
const ZF: u64 = 1 << 6;
const SF: u64 = 1 << 7;
pub const TF: u64 = 1 << 8;
const IF: u64 = 1 << 9;
const DF: u64 = 1 << 10;
const OF: u64 = 1 << 11;
const IOPL: u64 = 3 << 12;
const NT: u64 = 1 << 14;
pub const RF: u64 = 1 << 16;
const AC: u64 = 1 << 18;
const ID: u64 = 1 << 21;

/// `IA32_FMASK`: o que o Linux limpa na entrada (`idt_syscall_init`), `IF` e `AC` inclusive: a
/// entrada roda mascarada e o SMAP não precisa de `clac`.
pub const FMASK: u64 = CF | PF | AF | ZF | SF | TF | IF | DF | OF | IOPL | NT | RF | AC | ID;

/// `RFLAGS` com que um processo nasce: só `IF` (e o bit 1 reservado); `IOPL = 0`.
pub const USER_RFLAGS: u64 = 0x202;

/// Pode sair por `SYSRET`: `RCX == RIP` e `R11 == RFLAGS` (o `SYSRET` só sabe esses dois),
/// seletores de usuário, `RIP` abaixo de `USER_TOP` (canônico em toda CPU: CVE-2012-0217) e
/// `RFLAGS` sem `RF` nem `TF`. Qualquer dúvida sai por `IRETQ`.
#[allow(
    clippy::too_many_arguments,
    reason = "um argumento por campo do quadro que a conferência olha"
)]
pub const fn can_sysret(rcx: u64, r11: u64, rip: u64, rflags: u64, cs: u64, ss: u64) -> bool {
    rcx == rip
        && r11 == rflags
        && cs == USER_CODE as u64
        && ss == USER_DATA as u64
        && rip < USER_TOP
        && rflags & (RF | TF) == 0
}

/// `RIP` ou `RSP` do quadro que o `IRETQ` pode restaurar sem falhar no kernel: canônico de 48
/// bits. Não canônico no `IRETQ` seria `#GP` ou `#SS` já com o `GS` do usuário.
pub const fn is_canonical(address: u64) -> bool {
    (address << 16).cast_signed() >> 16 == address.cast_signed()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: (u64, u64, u64, u64, u64, u64) = (
        0x40_1000,
        0x202,
        0x40_1000,
        0x202,
        USER_CODE as u64,
        USER_DATA as u64,
    );

    fn run(rcx: u64, rip: u64) -> bool {
        can_sysret(rcx, GOOD.1, rip, GOOD.3, GOOD.4, GOOD.5)
    }

    #[test]
    fn star_gives_the_selectors_of_the_gdt_and_fmask_clears_the_dangerous_flags() {
        assert_eq!(STAR >> 48, 0x10);
        assert_eq!((STAR >> 32) & 0xFFFF, 0x08);
        assert_eq!(((STAR >> 48) + 16) | 3, u64::from(USER_CODE));
        assert_eq!(((STAR >> 48) + 8) | 3, u64::from(USER_DATA));
        assert_eq!(FMASK, 0x0025_7FD5);
        for flag in [IF, AC, TF, NT, DF, IOPL, RF] {
            assert_eq!(FMASK & flag, flag);
        }
        assert_eq!(FMASK & 2, 0, "o bit 1 é reservado e fica ligado");
        assert_eq!(USER_RFLAGS & IF, IF);
        assert_eq!(USER_RFLAGS & IOPL, 0);
    }

    #[test]
    fn a_plain_user_return_may_use_sysret() {
        assert!(can_sysret(GOOD.0, GOOD.1, GOOD.2, GOOD.3, GOOD.4, GOOD.5));
    }

    #[test]
    fn rip_at_the_edges_of_the_user_half() {
        assert!(run(USER_TOP - 1, USER_TOP - 1));
        assert!(!run(USER_TOP, USER_TOP));
        assert!(!run(0x0000_8000_0000_0000, 0x0000_8000_0000_0000));
        assert!(!run(0xFFFF_8000_0000_0000, 0xFFFF_8000_0000_0000));
        assert!(!run(u64::MAX, u64::MAX));
    }

    #[test]
    fn a_frame_the_kernel_changed_goes_through_iretq() {
        assert!(!can_sysret(
            0x40_1000, GOOD.1, 0x40_2000, GOOD.3, GOOD.4, GOOD.5
        ));
        assert!(!can_sysret(GOOD.0, 0x246, GOOD.2, GOOD.3, GOOD.4, GOOD.5));
        assert!(!can_sysret(GOOD.0, GOOD.1, GOOD.2, GOOD.3, 0x08, GOOD.5));
        assert!(!can_sysret(GOOD.0, GOOD.1, GOOD.2, GOOD.3, GOOD.4, 0x10));
    }

    #[test]
    fn resume_and_trap_flags_force_iretq() {
        let with = |flags: u64| can_sysret(GOOD.0, flags, GOOD.2, flags, GOOD.4, GOOD.5);
        assert!(with(0x202));
        assert!(!with(0x202 | RF));
        assert!(!with(0x202 | TF));
        assert!(
            with(0x202 | AC | NT),
            "o IRETQ e o SYSRET restauram AC e NT sem problema"
        );
    }

    #[test]
    fn canonical_addresses_are_the_ones_the_iretq_accepts() {
        for ok in [
            0,
            0x40_0000,
            0x0000_7FFF_FFFF_FFFF,
            0xFFFF_8000_0000_0000,
            u64::MAX,
        ] {
            assert!(is_canonical(ok), "{ok:#x}");
        }
        for bad in [
            0x0000_8000_0000_0000,
            0x0001_0000_0000_0000,
            0xFFFF_7FFF_FFFF_FFFF,
            0x8000_0000_0000_0000,
        ] {
            assert!(!is_canonical(bad), "{bad:#x}");
        }
    }
}
