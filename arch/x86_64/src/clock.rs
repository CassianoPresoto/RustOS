//! Relógio monotônico sobre o TSC já calibrado. Só aritmética inteira.

const NS_PER_SECOND: u128 = 1_000_000_000;

/// Converte ticks do TSC em nanossegundos; satura em `u64::MAX`. `hz` zero dá 0.
pub fn ticks_to_ns(ticks: u64, hz: u64) -> u64 {
    if hz == 0 {
        return 0;
    }
    let ns = u128::from(ticks) * NS_PER_SECOND / u128::from(hz);
    u64::try_from(ns).unwrap_or(u64::MAX)
}

/// Ticks de `hz` que `ns` nanossegundos valem; satura em `u64::MAX`.
pub fn ns_to_ticks(ns: u64, hz: u64) -> u64 {
    let ticks = u128::from(ns) * u128::from(hz) / NS_PER_SECOND;
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

/// O valor absoluto do TSC-deadline para `ticks` depois da origem: satura em `u64::MAX`, que é
/// "nunca". Com `wrapping_add` um prazo enorme dava a volta e armava o timer no passado.
pub const fn absolute_deadline(origin: u64, ticks: u64) -> u64 {
    origin.saturating_add(ticks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_huge_deadline_saturates_instead_of_wrapping_into_the_past() {
        let origin = 9_000_000_000_000;
        assert_eq!(absolute_deadline(origin, 1_000), origin + 1_000);
        assert_eq!(absolute_deadline(origin, u64::MAX), u64::MAX);
        assert_eq!(
            absolute_deadline(origin, ns_to_ticks(u64::MAX, 3_000_000_000)),
            u64::MAX
        );
    }

    #[test]
    fn one_second_of_ticks_is_a_billion_ns() {
        assert_eq!(ticks_to_ns(2_995_200_000, 2_995_200_000), 1_000_000_000);
    }

    #[test]
    fn conversion_does_not_overflow_u64_intermediate() {
        let hz = 3_000_000_000;
        let one_hour = 3_600 * hz;
        assert_eq!(ticks_to_ns(one_hour, hz), 3_600 * 1_000_000_000);
    }

    #[test]
    fn saturates_instead_of_wrapping() {
        assert_eq!(ticks_to_ns(u64::MAX, 1), u64::MAX);
    }

    #[test]
    fn ns_convert_to_ticks_at_62_5_mhz() {
        assert_eq!(ns_to_ticks(1_000, 62_500_000), 62);
        assert_eq!(ns_to_ticks(1_000_000_000, 62_500_000), 62_500_000);
        assert_eq!(ns_to_ticks(u64::MAX, u64::MAX), u64::MAX);
    }

    #[test]
    fn zero_frequency_reads_as_zero() {
        assert_eq!(ticks_to_ns(1_000, 0), 0);
    }
}
