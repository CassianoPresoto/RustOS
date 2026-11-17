//! Formatação sem alocação e sem ponto flutuante: tamanhos, porcentagens e tempo.

use core::fmt;

/// Texto de capacidade fixa; estourar a capacidade devolve `fmt::Error`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StackStr<const N: usize> {
    buf: [u8; N],
    len: usize,
}

impl<const N: usize> StackStr<N> {
    pub(crate) const fn new() -> Self {
        Self {
            buf: [0; N],
            len: 0,
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

impl<const N: usize> fmt::Write for StackStr<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len + s.len();
        if end > N {
            return Err(fmt::Error);
        }
        self.buf[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// Bytes em unidade legível: `B`, `KiB`, `MiB`, `GiB` ou `TiB`, com uma casa truncada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Bytes(pub u64);

const UNITS: [(&str, u32); 4] = [("TiB", 40), ("GiB", 30), ("MiB", 20), ("KiB", 10)];

impl fmt::Display for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut text = StackStr::<24>::new();
        let n = self.0;
        let unit = UNITS.iter().find(|&&(_, shift)| n >= 1 << shift);
        let written = match unit {
            Some(&(name, shift)) => {
                let remainder = n & ((1 << shift) - 1);
                let tenth = (remainder * 10) >> shift;
                fmt::write(&mut text, format_args!("{}.{tenth} {name}", n >> shift))
            }
            None => fmt::write(&mut text, format_args!("{n} B")),
        };
        written?;
        f.pad(text.as_str())
    }
}

/// `part` sobre `whole` em porcentagem, com `decimals` casas truncadas (no máximo 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Percent {
    pub(crate) part: u64,
    pub(crate) whole: u64,
    pub(crate) decimals: u32,
}

impl fmt::Display for Percent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let decimals = self.decimals.min(6);
        let scale = 10u64.pow(decimals);
        let factor = 100 * scale;
        let (mut part, mut whole) = (self.part, self.whole);
        while part > u64::MAX / factor {
            part >>= 1;
            whole >>= 1;
        }
        let ratio = part * factor / whole.max(1);
        let width = usize::try_from(decimals).unwrap_or(0);
        if width == 0 {
            return write!(f, "{}%", ratio / scale);
        }
        write!(f, "{}.{:0>width$}%", ratio / scale, ratio % scale)
    }
}

/// Microssegundos em milissegundos com uma casa truncada; o texto não leva unidade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Millis(pub u64);

impl fmt::Display for Millis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.0 / 1000, self.0 % 1000 / 100)
    }
}

/// Duração em nanossegundos, no formato do `uptime`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Uptime(pub u64);

impl fmt::Display for Uptime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let millis = self.0 / 1_000_000;
        let secs = millis / 1000;
        let (days, hours) = (secs / 86_400, secs % 86_400 / 3600);
        let (minutes, seconds) = (secs % 3600 / 60, secs % 60);
        if days > 0 {
            write!(f, "{days}d {hours}h {minutes:02}m {seconds:02}s")
        } else if hours > 0 {
            write!(f, "{hours}h {minutes:02}m {seconds:02}s")
        } else if minutes > 0 {
            write!(f, "{minutes}m {seconds:02}s")
        } else {
            write!(f, "{seconds}.{:03}s", millis % 1000)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write;

    #[test]
    fn stack_str_collects_writes() {
        let mut text = StackStr::<8>::new();
        write!(text, "ab{}", 12).unwrap();
        assert_eq!(text.as_str(), "ab12");
    }

    #[test]
    fn stack_str_rejects_overflow_and_keeps_content() {
        let mut text = StackStr::<4>::new();
        text.write_str("abc").unwrap();
        assert!(text.write_str("de").is_err());
        assert_eq!(text.as_str(), "abc");
    }

    #[test]
    fn stack_str_new_is_empty() {
        assert_eq!(StackStr::<4>::new().as_str(), "");
    }

    #[test]
    fn bytes_below_one_kib_are_plain() {
        assert_eq!(Bytes(0).to_string(), "0 B");
        assert_eq!(Bytes(1023).to_string(), "1023 B");
    }

    #[test]
    fn bytes_switch_unit_at_exact_powers() {
        assert_eq!(Bytes(1024).to_string(), "1.0 KiB");
        assert_eq!(Bytes(1 << 20).to_string(), "1.0 MiB");
        assert_eq!(Bytes(1 << 30).to_string(), "1.0 GiB");
        assert_eq!(Bytes(1 << 40).to_string(), "1.0 TiB");
    }

    #[test]
    fn bytes_truncate_to_one_decimal() {
        assert_eq!(Bytes(1536).to_string(), "1.5 KiB");
        assert_eq!(Bytes((1 << 20) - 1).to_string(), "1023.9 KiB");
        assert_eq!(Bytes(536_870_912 + 53_687_091).to_string(), "563.1 MiB");
    }

    #[test]
    fn bytes_handle_the_largest_value() {
        assert_eq!(Bytes(u64::MAX).to_string(), "16777215.9 TiB");
    }

    #[test]
    fn bytes_honour_width_and_alignment() {
        assert_eq!(format!("[{:>9}]", Bytes(1536)), "[  1.5 KiB]");
        assert_eq!(format!("[{:<9}]", Bytes(5)), "[5 B      ]");
    }

    #[test]
    fn percent_truncates_to_requested_decimals() {
        let pct = |part, whole, decimals| {
            Percent {
                part,
                whole,
                decimals,
            }
            .to_string()
        };
        assert_eq!(pct(1, 3, 0), "33%");
        assert_eq!(pct(1, 3, 1), "33.3%");
        assert_eq!(pct(2, 3, 1), "66.6%");
        assert_eq!(pct(1, 32_768, 3), "0.003%");
        assert_eq!(pct(5, 5, 2), "100.00%");
    }

    #[test]
    fn percent_of_nothing_is_zero() {
        let text = Percent {
            part: 0,
            whole: 0,
            decimals: 1,
        }
        .to_string();
        assert_eq!(text, "0.0%");
    }

    #[test]
    fn percent_does_not_overflow_on_huge_values() {
        let text = Percent {
            part: u64::MAX / 2,
            whole: u64::MAX,
            decimals: 3,
        }
        .to_string();
        assert_eq!(text, "49.999%");
    }

    #[test]
    fn percent_clamps_decimals_to_six() {
        let text = Percent {
            part: 1,
            whole: 4,
            decimals: 99,
        }
        .to_string();
        assert_eq!(text, "25.000000%");
    }

    #[test]
    fn millis_keep_one_truncated_decimal() {
        assert_eq!(Millis(0).to_string(), "0.0");
        assert_eq!(Millis(312_400).to_string(), "312.4");
        assert_eq!(Millis(8_199).to_string(), "8.1");
        assert_eq!(Millis(99).to_string(), "0.0");
    }

    #[test]
    fn uptime_under_a_minute_shows_milliseconds() {
        assert_eq!(Uptime(0).to_string(), "0.000s");
        assert_eq!(Uptime(12_345_678_901).to_string(), "12.345s");
    }

    #[test]
    fn uptime_grows_units_without_leading_zeroes() {
        assert_eq!(Uptime(65 * 1_000_000_000).to_string(), "1m 05s");
        assert_eq!(Uptime(3_723 * 1_000_000_000).to_string(), "1h 02m 03s");
        assert_eq!(Uptime(90_061 * 1_000_000_000).to_string(), "1d 1h 01m 01s");
    }
}
