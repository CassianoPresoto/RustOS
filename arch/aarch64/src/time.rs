//! Conversões puras do contador genérico: ticks ↔ nanossegundos, só com inteiros.

const NS_PER_SECOND: u128 = 1_000_000_000;

/// Nanossegundos que `ticks` valem a `frequency_hz`; satura em `u64::MAX`.
/// `None` com frequência zero (`CNTFRQ_EL0` que o firmware não programou).
pub fn ticks_to_ns(ticks: u64, frequency_hz: u64) -> Option<u64> {
    let ns = (u128::from(ticks) * NS_PER_SECOND).checked_div(u128::from(frequency_hz))?;
    Some(u64::try_from(ns).unwrap_or(u64::MAX))
}

/// Ticks que `ns` nanossegundos valem a `frequency_hz`; satura em `u64::MAX`.
pub fn ns_to_ticks(ns: u64, frequency_hz: u64) -> u64 {
    let ticks = u128::from(ns) * u128::from(frequency_hz) / NS_PER_SECOND;
    u64::try_from(ticks).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_convert_at_24_mhz() {
        assert_eq!(ticks_to_ns(24_000_000, 24_000_000), Some(1_000_000_000));
        assert_eq!(ticks_to_ns(240_000, 24_000_000), Some(10_000_000));
        assert_eq!(ticks_to_ns(1, 24_000_000), Some(41));
    }

    #[test]
    fn counts_convert_at_1_ghz_without_loss() {
        assert_eq!(ticks_to_ns(123_456_789, 1_000_000_000), Some(123_456_789));
    }

    #[test]
    fn conversion_keeps_precision_for_a_huge_count_and_saturates_past_u64() {
        assert_eq!(ticks_to_ns(1 << 60, 1_000_000_000), Some(1 << 60));
        assert_eq!(ticks_to_ns(u64::MAX, 24_000_000), Some(u64::MAX));
    }

    #[test]
    fn zero_frequency_has_no_time() {
        assert_eq!(ticks_to_ns(1000, 0), None);
        assert_eq!(ns_to_ticks(1000, 0), 0);
    }

    #[test]
    fn nanoseconds_convert_to_ticks_at_24_mhz() {
        assert_eq!(ns_to_ticks(1_000_000_000, 24_000_000), 24_000_000);
        assert_eq!(ns_to_ticks(10_000_000, 24_000_000), 240_000);
        assert_eq!(ns_to_ticks(u64::MAX, u64::MAX), u64::MAX);
    }
}
