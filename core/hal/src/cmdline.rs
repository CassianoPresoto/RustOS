//! A linha de comando do kernel (`cmdline:` do Limine) traduzida para o que o boot entende.
//! Chave desconhecida ou valor inválido é ignorado: o boot nunca falha por causa dela.
//! As chaves e o que cada uma liga estão no SDD 04.

use crate::mitigation::KptiPolicy;

/// Onde `rustos.panic_early` derruba o boot, para provar que o pânico sai sem console.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EarlyPanic {
    /// `1`: pânico logo depois de ler o bootloader, antes da memória.
    Entry,
    /// `2`: page fault no mesmo ponto, antes de a IDT ou o `VBAR` serem os do kernel.
    Fault,
    /// `3`: pânico com as tabelas próprias já ativas e o console ainda por montar.
    Activated,
}

impl EarlyPanic {
    fn from_value(value: &str) -> Option<Self> {
        match value {
            "1" => Some(Self::Entry),
            "2" => Some(Self::Fault),
            "3" => Some(Self::Activated),
            _ => None,
        }
    }
}

/// O que a linha de comando pede ao kernel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BootParams {
    pub early_panic: Option<EarlyPanic>,
    /// `rustos.selftest=1`: o boot roda os autotestes e os benchmarks completos.
    pub selftest: bool,
    /// `kpti=on|off|auto`: o que o relatório diz que o KPTI faria (SDD 17).
    pub kpti: KptiPolicy,
}

impl BootParams {
    /// Lê `chave=valor` separados por espaço; a última ocorrência de uma chave vale.
    pub fn parse(line: &str) -> Self {
        let mut params = Self::default();
        for word in line.split_ascii_whitespace() {
            match word.split_once('=') {
                Some(("rustos.panic_early", value)) => {
                    params.early_panic = EarlyPanic::from_value(value);
                }
                Some(("rustos.selftest", value)) => params.selftest = value == "1",
                Some(("kpti", value)) => {
                    params.kpti = KptiPolicy::from_value(value).unwrap_or_default();
                }
                _ => {}
            }
        }
        params
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_early_panic_point_comes_from_its_key() {
        for (line, expected) in [
            ("rustos.panic_early=1", Some(EarlyPanic::Entry)),
            ("quiet rustos.panic_early=2 x=y", Some(EarlyPanic::Fault)),
            ("rustos.panic_early=3", Some(EarlyPanic::Activated)),
        ] {
            assert_eq!(BootParams::parse(line).early_panic, expected, "{line}");
        }
    }

    #[test]
    fn what_is_not_understood_leaves_the_defaults() {
        for line in [
            "",
            "rustos.panic_early",
            "rustos.panic_early=",
            "rustos.panic_early=9",
            "rustos.panic_early=01",
            "rustos.panic_early=one",
            "rustos.panic_early_x=1",
            "xrustos.panic_early=1",
            "rustos.panic_early =1",
            "\u{e9}=1 \u{0}",
        ] {
            assert_eq!(BootParams::parse(line), BootParams::default(), "{line:?}");
        }
    }

    #[test]
    fn the_self_test_mode_is_off_unless_asked_for() {
        assert!(!BootParams::parse("").selftest);
        assert!(BootParams::parse("rustos.selftest=1").selftest);
        assert!(BootParams::parse("quiet rustos.selftest=1 rustos.panic_early=2").selftest);
        for line in [
            "rustos.selftest",
            "rustos.selftest=",
            "rustos.selftest=0",
            "rustos.selftest=true",
            "rustos.selftest=11",
            "xrustos.selftest=1",
            "rustos.selftest =1",
        ] {
            assert!(!BootParams::parse(line).selftest, "{line:?}");
        }
    }

    #[test]
    fn the_kpti_key_picks_the_policy_and_an_invalid_value_means_auto() {
        assert_eq!(BootParams::parse("").kpti, KptiPolicy::Auto);
        for (line, expected) in [
            ("kpti=on", KptiPolicy::On),
            ("quiet kpti=off rustos.selftest=1", KptiPolicy::Off),
            ("kpti=auto", KptiPolicy::Auto),
            ("kpti=on kpti=off", KptiPolicy::Off),
            ("kpti=off kpti=x", KptiPolicy::Auto),
            ("kpti=", KptiPolicy::Auto),
            ("kpti=ON", KptiPolicy::Auto),
            ("kpti", KptiPolicy::Auto),
            ("xkpti=on", KptiPolicy::Auto),
            ("rustos.kpti=on", KptiPolicy::Auto),
        ] {
            assert_eq!(BootParams::parse(line).kpti, expected, "{line:?}");
        }
        assert!(BootParams::parse("kpti=on rustos.selftest=1").selftest);
    }

    #[test]
    fn the_self_test_key_does_not_disturb_the_early_panic_key() {
        let params = BootParams::parse("rustos.selftest=1 rustos.panic_early=3");
        assert_eq!(params.early_panic, Some(EarlyPanic::Activated));
        assert!(params.selftest);
    }

    #[test]
    fn the_last_occurrence_wins_even_when_it_is_invalid() {
        assert!(!BootParams::parse("rustos.selftest=1 rustos.selftest=x").selftest);
        let params = BootParams::parse("rustos.panic_early=1 rustos.panic_early=x");
        assert_eq!(params.early_panic, None);
        let params = BootParams::parse("rustos.panic_early=x rustos.panic_early=3");
        assert_eq!(params.early_panic, Some(EarlyPanic::Activated));
    }
}
