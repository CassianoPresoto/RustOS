//! A linha `mitigations:` do boot: o que a CPU tem de vulnerabilidade e o que o kernel liga
//! (SDD 09 e SDD 17, "Mitigações de CPU").

use rustos_hal::mitigation::{Enabled, KptiPolicy, Mitigations, Vulns};
use rustos_sync::SpinLock;

use crate::platform;

/// O pior de cada vulnerabilidade entre os núcleos que já rodaram `note_cpu`, como o Linux, que
/// avalia cada CPU e une.
static UNION: SpinLock<Vulns> = SpinLock::new(Vulns::NONE);

/// Soma a vulnerabilidade deste núcleo: o de boot e cada secundário, antes de ficar online.
pub fn note_cpu() {
    let own = platform::mitigation_features().vulnerabilities();
    let mut union = UNION.lock();
    *union = union.union(own);
}

/// Só o clamp do Spectre v1 está ligado; o resto sai `not implemented` onde a CPU precisa
/// (DEBT-0100 a DEBT-0104). Os secundários já subiram e se somaram (DEBT-0107 para os que não).
pub fn report(policy: KptiPolicy) {
    note_cpu();
    let vulns = *UNION.lock();
    kprintln!(
        "[rustos] mitigations: {}",
        Mitigations::decide(vulns, policy, Enabled::default())
    );
}

/// O que o CPUID ou os registradores de identificação anunciaram no núcleo de boot (só o modo
/// de teste).
#[cfg(feature = "selftest")]
pub fn details() {
    kprintln!("[rustos] cpu features: {}", platform::mitigation_features());
}
