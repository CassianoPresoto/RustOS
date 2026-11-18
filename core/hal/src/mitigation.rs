//! O que a CPU tem de vulnerabilidade, a política do KPTI e a linha `mitigations:` do boot.
//! Puro: quem decodifica os registradores é o adaptador de cada ISA (SDD 17, "Mitigações de CPU").

use core::fmt;

/// As vulnerabilidades que a CPU tem, pela regra do Linux; sem informação, vale `true`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "uma vulnerabilidade por campo"
)]
pub struct Vulns {
    /// Meltdown (CVE-2017-5754): o KPTI é a resposta.
    pub meltdown: bool,
    /// Spectre v2: desvio indireto envenenado.
    pub spectre_v2: bool,
    /// Spectre v4: desvio de armazenamento especulativo (`SSBD`/`SSBS`).
    pub ssb: bool,
    /// Amostragem de dados de microarquitetura (MDS): só Intel.
    pub mds: bool,
    /// Injeção de histórico de desvio (BHI no x86_64, Spectre-BHB no aarch64).
    pub bhb: bool,
}

impl Vulns {
    /// Toda vulnerabilidade presente: o que vale sem informação da CPU.
    pub const ALL: Self = Self {
        meltdown: true,
        spectre_v2: true,
        ssb: true,
        mds: true,
        bhb: true,
    };
}

/// `kpti=on|off|auto` na linha de comando; sem a chave, `Auto`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KptiPolicy {
    /// Liga só se a CPU tem Meltdown.
    #[default]
    Auto,
    On,
    Off,
}

impl KptiPolicy {
    /// `None` para valor desconhecido: o boot ignora e fica com `Auto`.
    pub fn from_value(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "on" => Some(Self::On),
            "off" => Some(Self::Off),
            _ => None,
        }
    }

    /// Se o kernel ligaria o KPTI nesta CPU com esta política.
    pub const fn wanted(self, vulns: Vulns) -> bool {
        match self {
            Self::Auto => vulns.meltdown,
            Self::On => true,
            Self::Off => false,
        }
    }
}

/// O que o kernel de fato liga de cada mitigação; hoje nenhuma além do clamp do Spectre v1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Switch {
    #[default]
    Off,
    On,
    /// Ligada, mas nenhum ambiente exercita (QEMU não modela): vira dívida com gatilho.
    Untested,
}

/// A mitigação por vulnerabilidade, na ordem da linha.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Enabled {
    pub kpti: Switch,
    pub spectre_v2: Switch,
    pub ssb: Switch,
    pub mds: Switch,
    pub bhb: Switch,
}

/// Como uma mitigação aparece na linha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    On,
    /// A CPU não tem a vulnerabilidade.
    NotNeeded,
    /// A CPU precisa e o kernel ainda não tem a mitigação.
    NotImplemented,
    Untested,
}

impl Status {
    const fn of(vulnerable: bool, switch: Switch) -> Self {
        match (vulnerable, switch) {
            (false, _) => Self::NotNeeded,
            (true, Switch::Off) => Self::NotImplemented,
            (true, Switch::On) => Self::On,
            (true, Switch::Untested) => Self::Untested,
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::On => "on",
            Self::NotNeeded => "off (not needed)",
            Self::NotImplemented => "off (not implemented)",
            Self::Untested => "off (untested)",
        })
    }
}

/// O KPTI na linha: além do estado, o que o kernel faria com a política escolhida.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KptiStatus {
    On,
    Untested,
    NotNeeded,
    /// A CPU tem Meltdown e `kpti=off` mandou não ligar.
    ForcedOff,
    /// O kernel ligaria e ainda não sabe; `forced` quando foi `kpti=on` e a CPU não precisa.
    WouldBeOn {
        forced: bool,
    },
}

impl fmt::Display for KptiStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::On => "on",
            Self::Untested => "off (untested)",
            Self::NotNeeded => "off (not needed)",
            Self::ForcedOff => "off (kpti=off)",
            Self::WouldBeOn { forced: false } => "off (not implemented, would be on)",
            Self::WouldBeOn { forced: true } => "off (not implemented, would be on by kpti=on)",
        })
    }
}

/// A linha `mitigations:` inteira: o clamp do Spectre v1 está sempre ligado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mitigations {
    pub kpti: KptiStatus,
    pub spectre_v2: Status,
    pub ssb: Status,
    pub mds: Status,
    pub bhb: Status,
}

impl Mitigations {
    pub const fn decide(vulns: Vulns, policy: KptiPolicy, enabled: Enabled) -> Self {
        let kpti = if policy.wanted(vulns) {
            match enabled.kpti {
                Switch::On => KptiStatus::On,
                Switch::Untested => KptiStatus::Untested,
                Switch::Off => KptiStatus::WouldBeOn {
                    forced: !vulns.meltdown,
                },
            }
        } else if vulns.meltdown {
            KptiStatus::ForcedOff
        } else {
            KptiStatus::NotNeeded
        };
        Self {
            kpti,
            spectre_v2: Status::of(vulns.spectre_v2, enabled.spectre_v2),
            ssb: Status::of(vulns.ssb, enabled.ssb),
            mds: Status::of(vulns.mds, enabled.mds),
            bhb: Status::of(vulns.bhb, enabled.bhb),
        }
    }
}

impl fmt::Display for Mitigations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "v1 on, kpti {}, v2 {}, ssb {}, mds {}, bhb {}",
            self.kpti, self.spectre_v2, self.ssb, self.mds, self.bhb
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_policy_comes_from_the_value() {
        assert_eq!(KptiPolicy::from_value("auto"), Some(KptiPolicy::Auto));
        assert_eq!(KptiPolicy::from_value("on"), Some(KptiPolicy::On));
        assert_eq!(KptiPolicy::from_value("off"), Some(KptiPolicy::Off));
        for value in ["", "ON", "1", "yes", "on ", "autoo"] {
            assert_eq!(KptiPolicy::from_value(value), None, "{value:?}");
        }
    }

    #[test]
    fn auto_follows_meltdown_and_the_override_wins() {
        let (vulnerable, immune) = (Vulns::ALL, Vulns::default());
        assert!(KptiPolicy::Auto.wanted(vulnerable));
        assert!(!KptiPolicy::Auto.wanted(immune));
        assert!(KptiPolicy::On.wanted(immune));
        assert!(!KptiPolicy::Off.wanted(vulnerable));
    }

    #[test]
    fn a_vulnerable_cpu_with_nothing_implemented_says_not_implemented() {
        let line = Mitigations::decide(Vulns::ALL, KptiPolicy::Auto, Enabled::default());
        assert_eq!(
            line.to_string(),
            "v1 on, kpti off (not implemented, would be on), v2 off (not implemented), \
             ssb off (not implemented), mds off (not implemented), bhb off (not implemented)"
        );
    }

    #[test]
    fn an_immune_cpu_says_not_needed() {
        let line = Mitigations::decide(Vulns::default(), KptiPolicy::Auto, Enabled::default());
        assert_eq!(
            line.to_string(),
            "v1 on, kpti off (not needed), v2 off (not needed), ssb off (not needed), \
             mds off (not needed), bhb off (not needed)"
        );
    }

    #[test]
    fn the_kpti_override_changes_only_the_kpti_part() {
        let on = Mitigations::decide(Vulns::default(), KptiPolicy::On, Enabled::default());
        assert_eq!(
            on.kpti.to_string(),
            "off (not implemented, would be on by kpti=on)"
        );
        let off = Mitigations::decide(Vulns::ALL, KptiPolicy::Off, Enabled::default());
        assert_eq!(off.kpti.to_string(), "off (kpti=off)");
        assert_eq!(off.spectre_v2, Status::NotImplemented);
        let immune_off = Mitigations::decide(Vulns::default(), KptiPolicy::Off, Enabled::default());
        assert_eq!(immune_off.kpti, KptiStatus::NotNeeded);
    }

    #[test]
    fn what_the_kernel_turns_on_shows_as_on_or_untested() {
        let enabled = Enabled {
            kpti: Switch::On,
            spectre_v2: Switch::Untested,
            ssb: Switch::On,
            mds: Switch::Untested,
            bhb: Switch::Off,
        };
        let line = Mitigations::decide(Vulns::ALL, KptiPolicy::Auto, enabled);
        assert_eq!(
            line.to_string(),
            "v1 on, kpti on, v2 off (untested), ssb on, mds off (untested), bhb off (not implemented)"
        );
        let untested = Enabled {
            kpti: Switch::Untested,
            ..Enabled::default()
        };
        let line = Mitigations::decide(Vulns::ALL, KptiPolicy::Auto, untested);
        assert_eq!(line.kpti, KptiStatus::Untested);
    }

    #[test]
    fn a_mitigation_the_cpu_does_not_need_stays_not_needed_even_when_enabled() {
        let enabled = Enabled {
            mds: Switch::On,
            ..Enabled::default()
        };
        let line = Mitigations::decide(Vulns::default(), KptiPolicy::Auto, enabled);
        assert_eq!(line.mds, Status::NotNeeded);
    }
}
