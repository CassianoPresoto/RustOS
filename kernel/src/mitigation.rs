//! A linha `mitigations:` do boot: o que a CPU tem de vulnerabilidade e o que o kernel liga
//! (SDD 09 e SDD 17, "Mitigações de CPU").

use rustos_hal::mitigation::{Enabled, KptiPolicy, Mitigations};

use crate::platform;

/// Só o clamp do Spectre v1 está ligado; o resto sai `not implemented` onde a CPU precisa
/// (DEBT-0100 a DEBT-0104).
pub fn report(policy: KptiPolicy) {
    let vulns = platform::mitigation_features().vulnerabilities();
    kprintln!(
        "[rustos] mitigations: {}",
        Mitigations::decide(vulns, policy, Enabled::default())
    );
}

/// O que o CPUID ou os registradores de identificação anunciaram (só o modo de teste).
#[cfg(feature = "selftest")]
pub fn details() {
    kprintln!("[rustos] cpu features: {}", platform::mitigation_features());
}
