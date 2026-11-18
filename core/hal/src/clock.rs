//! Conversão de ticks de contador para nanossegundos, só com inteiros (SDD 15, "Tempo").

const NS_PER_SECOND: u128 = 1_000_000_000;

/// `ns = ticks * mult >> shift`, com `mult` arredondado para cima: erro abaixo de 1 ns até
/// `2^shift` ticks. `ZERO` é o relógio que ainda não foi ligado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickScale {
    mult: u64,
    shift: u32,
}

impl TickScale {
    pub const ZERO: Self = Self { mult: 0, shift: 0 };

    /// `None` com frequência zero. Usa o maior `shift` que deixa `mult` em `u64`.
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

    /// Os campos, para guardá-los em atômicos: grave o `shift` antes do `mult`.
    pub const fn parts(self) -> (u64, u32) {
        (self.mult, self.shift)
    }

    pub const fn from_parts(mult: u64, shift: u32) -> Self {
        Self { mult, shift }
    }

    pub const fn is_zero(self) -> bool {
        self.mult == 0
    }

    /// Nanossegundos de `ticks`; satura em `u64::MAX`.
    pub fn convert(self, ticks: u64) -> u64 {
        let ns = (u128::from(ticks) * u128::from(self.mult)) >> self.shift;
        u64::try_from(ns).unwrap_or(u64::MAX)
    }

    /// Nanossegundos desde `origin`; um contador atrás da origem lê zero.
    pub fn elapsed_ns(self, now: u64, origin: u64) -> u64 {
        self.convert(now.saturating_sub(origin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exact(ticks: u64, hz: u64) -> u64 {
        let ns = u128::from(ticks) * NS_PER_SECOND / u128::from(hz);
        u64::try_from(ns).unwrap_or(u64::MAX)
    }

    #[test]
    fn scale_matches_the_division_within_one_ns() {
        let frequencies = [
            1,
            1_000,
            32_768,
            1_000_000,
            10_000_000,
            19_200_000,
            24_000_000,
            25_000_000,
            62_500_000,
            100_000_000,
            400_000_000,
            999_999_999,
            1_000_000_000,
            2_995_200_000,
            3_000_000_000,
            5_000_000_000,
            1 << 40,
        ];
        for hz in frequencies {
            let scale = TickScale::new(hz).unwrap();
            let year = hz.saturating_mul(86_400 * 365);
            for ticks in [0, 1, 7, 999, hz, hz.saturating_mul(3_600), year, 1 << 50] {
                let (fast, slow) = (scale.convert(ticks), exact(ticks, hz));
                assert!(
                    fast >= slow && fast - slow <= 1,
                    "{hz} Hz, {ticks}: {fast} x {slow}"
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

    #[test]
    fn elapsed_clamps_a_counter_behind_the_origin() {
        let scale = TickScale::new(1_000_000_000).unwrap();
        assert_eq!(scale.elapsed_ns(10, 20), 0);
        assert_eq!(scale.elapsed_ns(30, 20), 10);
    }
}
