//! A linha de comando do kernel (`cmdline:` do Limine) traduzida para o que o boot entende.
//! Chave desconhecida ou valor inválido é ignorado: o boot nunca falha por causa dela.

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
}

impl BootParams {
    /// Lê `chave=valor` separados por espaço; a última ocorrência de uma chave vale.
    pub fn parse(line: &str) -> Self {
        let mut params = Self::default();
        for word in line.split_ascii_whitespace() {
            if let Some(("rustos.panic_early", value)) = word.split_once('=') {
                params.early_panic = EarlyPanic::from_value(value);
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
    fn the_last_occurrence_wins_even_when_it_is_invalid() {
        let params = BootParams::parse("rustos.panic_early=1 rustos.panic_early=x");
        assert_eq!(params.early_panic, None);
        let params = BootParams::parse("rustos.panic_early=x rustos.panic_early=3");
        assert_eq!(params.early_panic, Some(EarlyPanic::Activated));
    }
}
