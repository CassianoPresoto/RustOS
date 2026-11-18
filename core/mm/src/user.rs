//! Faixa de memória de usuário conferida: todo argumento de ponteiro de syscall vira um
//! `UserRange` antes de qualquer acesso, mesmo com SMAP e PAN ligados (SDD 17).

use rustos_hal::abi::{USER_MIN, USER_TOP};

/// Por que uma faixa não é de usuário.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserRangeError {
    /// Começa abaixo de `USER_MIN`: a faixa nula nunca mapeia.
    NullZone,
    /// `addr + len` transborda o endereço de 64 bits.
    Overflow,
    /// Termina acima de `USER_TOP`.
    AboveTop,
}

/// `[addr, addr + len)` dentro de `[USER_MIN, USER_TOP)`. Só a existência do valor prova isso.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserRange {
    addr: u64,
    len: u64,
}

impl UserRange {
    /// Recusa `addr` na faixa nula, `len` que estoura o endereço e fim acima de `USER_TOP`.
    /// Faixa vazia vale se o endereço dela vale.
    pub const fn new(addr: u64, len: u64) -> Result<Self, UserRangeError> {
        if addr < USER_MIN {
            return Err(UserRangeError::NullZone);
        }
        let Some(end) = addr.checked_add(len) else {
            return Err(UserRangeError::Overflow);
        };
        if end > USER_TOP {
            return Err(UserRangeError::AboveTop);
        }
        Ok(Self { addr, len })
    }

    pub const fn addr(self) -> u64 {
        self.addr
    }

    pub const fn len(self) -> u64 {
        self.len
    }

    pub const fn is_empty(self) -> bool {
        self.len == 0
    }

    /// Fim exclusivo; nunca transborda.
    pub const fn end(self) -> u64 {
        self.addr + self.len
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_inside_the_user_half_is_accepted() {
        let range = UserRange::new(USER_MIN, 4096).unwrap();
        assert_eq!(
            (range.addr(), range.len(), range.end()),
            (USER_MIN, 4096, USER_MIN + 4096)
        );
        assert!(UserRange::new(0x40_0000, 1).is_ok());
    }

    #[test]
    fn the_null_zone_is_refused() {
        assert_eq!(UserRange::new(0, 1), Err(UserRangeError::NullZone));
        assert_eq!(
            UserRange::new(USER_MIN - 1, 1),
            Err(UserRangeError::NullZone)
        );
        assert_eq!(UserRange::new(0, 0), Err(UserRangeError::NullZone));
    }

    #[test]
    fn the_last_byte_below_user_top_is_the_limit() {
        assert!(UserRange::new(USER_TOP - 1, 1).is_ok());
        assert_eq!(
            UserRange::new(USER_TOP - 1, 2),
            Err(UserRangeError::AboveTop)
        );
        assert_eq!(UserRange::new(USER_TOP, 1), Err(UserRangeError::AboveTop));
        assert!(UserRange::new(USER_TOP, 0).is_ok());
        assert_eq!(
            UserRange::new(USER_TOP + 1, 0),
            Err(UserRangeError::AboveTop)
        );
    }

    #[test]
    fn the_kernel_half_and_the_canonical_hole_are_refused() {
        for addr in [
            0x0000_8000_0000_0000,
            0xFFFF_8000_0000_0000,
            0xFFFF_FFFF_8000_0000,
        ] {
            assert_eq!(
                UserRange::new(addr, 1),
                Err(UserRangeError::AboveTop),
                "{addr:#x}"
            );
        }
    }

    #[test]
    fn a_length_that_overflows_the_address_is_refused() {
        assert_eq!(
            UserRange::new(USER_MIN, u64::MAX),
            Err(UserRangeError::Overflow)
        );
        assert_eq!(UserRange::new(u64::MAX, 2), Err(UserRangeError::Overflow));
        assert_eq!(
            UserRange::new(USER_MIN, u64::MAX - USER_MIN + 1),
            Err(UserRangeError::Overflow)
        );
        assert_eq!(
            UserRange::new(USER_MIN, u64::MAX - USER_MIN),
            Err(UserRangeError::AboveTop)
        );
    }

    #[test]
    fn an_empty_range_is_accepted_when_its_address_is() {
        assert!(UserRange::new(USER_MIN, 0).unwrap().is_empty());
    }
}
