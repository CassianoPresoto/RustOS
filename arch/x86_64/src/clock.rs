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
                let exact = ticks_to_ns(ticks, hz);
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
