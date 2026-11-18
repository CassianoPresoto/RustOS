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

/// Conversão de ticks para ns por multiplicação e deslocamento (o `cyc2ns` do Linux), sem a
/// divisão de 128 bits de `ticks_to_ns`: `ns = ticks * mult >> shift`, com `mult` arredondado
/// para cima. Erro: menos de 1 ns para até `2^shift` ticks, que são anos de contador.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickScale {
    mult: u64,
    shift: u32,
}

impl TickScale {
    /// Sem frequência: `convert` devolve zero.
    pub const ZERO: Self = Self { mult: 0, shift: 0 };

    /// `None` com frequência zero. O maior `shift` que deixa `mult` em `u64`.
    pub fn new(frequency_hz: u64) -> Option<Self> {
        if frequency_hz == 0 {
            return None;
        }
        let mut shift = 63;
        loop {
            let mult = (NS_PER_SECOND << shift).div_ceil(u128::from(frequency_hz));
            if let Ok(mult) = u64::try_from(mult) {
                return Some(Self { mult, shift });
            }
            shift -= 1;
        }
    }

    /// De volta aos campos, para guardá-los em atômicos.
    pub const fn parts(self) -> (u64, u32) {
        (self.mult, self.shift)
    }

    pub const fn from_parts(mult: u64, shift: u32) -> Self {
        Self { mult, shift }
    }

    /// Falta a frequência: o relógio ainda não foi ligado.
    pub const fn is_zero(self) -> bool {
        self.mult == 0
    }

    /// Nanossegundos de `ticks`; satura em `u64::MAX`.
    pub fn convert(self, ticks: u64) -> u64 {
        let ns = (u128::from(ticks) * u128::from(self.mult)) >> self.shift;
        u64::try_from(ns).unwrap_or(u64::MAX)
    }
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

    #[test]
    fn scale_matches_the_division_within_one_ns() {
        for hz in [
            24_000_000,
            62_500_000,
            1_000_000_000,
            2_995_200_000,
            3_000_000_000,
        ] {
            let scale = TickScale::new(hz).unwrap();
            for ticks in [0, 1, 7, 999, hz, hz * 3_600, hz * 86_400 * 365, 1 << 50] {
                let exact = ticks_to_ns(ticks, hz).unwrap_or(0);
                let fast = scale.convert(ticks);
                assert!(
                    fast >= exact && fast - exact <= 1,
                    "{hz} Hz, {ticks}: {fast} x {exact}"
                );
            }
        }
    }

    #[test]
    fn scale_is_exact_on_whole_seconds_and_monotonic() {
        let scale = TickScale::new(24_000_000).unwrap();
        assert_eq!(scale.convert(24_000_000), 1_000_000_000);
        assert_eq!(scale.convert(240_000), 10_000_000);
        let mut previous = 0;
        for ticks in (0..2_000_000).step_by(37) {
            let ns = scale.convert(ticks);
            assert!(ns >= previous);
            previous = ns;
        }
    }

    #[test]
    fn scale_saturates_and_has_no_zero_frequency() {
        assert_eq!(TickScale::new(1).unwrap().convert(u64::MAX), u64::MAX);
        assert_eq!(TickScale::new(0), None);
        assert!(TickScale::ZERO.is_zero());
        assert_eq!(TickScale::ZERO.convert(123), 0);
        let (mult, shift) = TickScale::new(62_500_000).unwrap().parts();
        assert_eq!(
            TickScale::from_parts(mult, shift).convert(62_500_000),
            1_000_000_000
        );
    }
}
