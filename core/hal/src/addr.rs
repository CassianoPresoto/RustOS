//! Endereços físicos e virtuais como tipos distintos: um não se passa pelo outro.

use core::fmt;

/// Arredonda `value` para baixo até um múltiplo de `align` (potência de dois).
pub const fn align_down(value: u64, align: u64) -> u64 {
    debug_assert!(align.is_power_of_two());
    value & !(align - 1)
}

/// Arredonda `value` para cima até um múltiplo de `align`; `None` se transborda.
pub const fn align_up(value: u64, align: u64) -> Option<u64> {
    debug_assert!(align.is_power_of_two());
    match value.checked_add(align - 1) {
        Some(bumped) => Some(bumped & !(align - 1)),
        None => None,
    }
}

macro_rules! address_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        #[repr(transparent)]
        pub struct $name(u64);

        impl $name {
            pub const fn new(addr: u64) -> Self {
                Self(addr)
            }

            pub const fn as_u64(self) -> u64 {
                self.0
            }

            #[must_use]
            pub const fn align_down(self, align: u64) -> Self {
                Self(align_down(self.0, align))
            }

            pub const fn align_up(self, align: u64) -> Option<Self> {
                match align_up(self.0, align) {
                    Some(addr) => Some(Self(addr)),
                    None => None,
                }
            }

            pub const fn is_aligned(self, align: u64) -> bool {
                align_down(self.0, align) == self.0
            }

            pub const fn checked_add(self, bytes: u64) -> Option<Self> {
                match self.0.checked_add(bytes) {
                    Some(addr) => Some(Self(addr)),
                    None => None,
                }
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({:#x})"), self.0)
            }
        }

        impl fmt::LowerHex for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::LowerHex::fmt(&self.0, f)
            }
        }
    };
}

address_type!(
    /// Endereço físico. Não é ponteiro: só vira acesso depois de mapeado.
    PhysAddr
);

address_type!(
    /// Endereço virtual no espaço do kernel.
    VirtAddr
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_down_clears_low_bits() {
        assert_eq!(align_down(0x1fff, 0x1000), 0x1000);
        assert_eq!(align_down(0x1000, 0x1000), 0x1000);
    }

    #[test]
    fn align_up_rounds_to_next_multiple() {
        assert_eq!(align_up(0x1001, 0x1000), Some(0x2000));
        assert_eq!(align_up(0x1000, 0x1000), Some(0x1000));
    }

    #[test]
    fn align_up_reports_overflow() {
        assert_eq!(align_up(u64::MAX, 0x1000), None);
    }

    #[test]
    fn address_alignment_helpers_agree() {
        let addr = PhysAddr::new(0x4321);
        assert!(!addr.is_aligned(0x1000));
        assert_eq!(addr.align_down(0x1000), PhysAddr::new(0x4000));
        assert_eq!(addr.align_up(0x1000), Some(PhysAddr::new(0x5000)));
    }

    #[test]
    fn debug_prints_hex_with_type_name() {
        assert_eq!(format!("{:?}", VirtAddr::new(0xff)), "VirtAddr(0xff)");
    }
}
