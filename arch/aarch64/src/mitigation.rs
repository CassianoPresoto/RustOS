//! Recursos de CPU das mitigações (`MIDR_EL1` e os registradores de identificação) e a regra do
//! Linux para decidir quais vulnerabilidades a CPU tem. Puro: a leitura dos registradores está
//! em `protection`.

use core::fmt;

use rustos_hal::mitigation::{Exposure, Vulns};

const ARM: u32 = 0x41;
const BRCM: u32 = 0x42;
const CAVIUM: u32 = 0x43;
const NVIDIA: u32 = 0x4E;
const HISI: u32 = 0x48;
const QCOM: u32 = 0x51;
const AMPERE: u32 = 0xC0;

/// `(implementador, número da peça)` de `cputype.h` do Linux v6.12.
type Model = (u32, u32);

const CORTEX_A35: Model = (ARM, 0xD04);
const CORTEX_A53: Model = (ARM, 0xD03);
const CORTEX_A55: Model = (ARM, 0xD05);
const BRAHMA_B53: Model = (BRCM, 0x100);
const KRYO_2XX_SILVER: Model = (QCOM, 0x801);
const KRYO_3XX_SILVER: Model = (QCOM, 0x803);
const KRYO_4XX_SILVER: Model = (QCOM, 0x805);
const TSV110: Model = (HISI, 0xD01);

/// `kpti_safe_list` de `cpufeature.c`: CPUs sem Meltdown.
const MELTDOWN_SAFE: [Model; 15] = [
    (CAVIUM, 0x0AF),
    (BRCM, 0x516),
    BRAHMA_B53,
    CORTEX_A35,
    CORTEX_A53,
    CORTEX_A55,
    (ARM, 0xD07),
    (ARM, 0xD08),
    (ARM, 0xD09),
    TSV110,
    (NVIDIA, 0x004),
    (QCOM, 0x800),
    KRYO_2XX_SILVER,
    KRYO_3XX_SILVER,
    KRYO_4XX_SILVER,
];

/// `spectre_v2_safe_list` de `proton-pack.c`.
const SPECTRE_V2_SAFE: [Model; 8] = [
    CORTEX_A35,
    CORTEX_A53,
    CORTEX_A55,
    BRAHMA_B53,
    TSV110,
    KRYO_2XX_SILVER,
    KRYO_3XX_SILVER,
    KRYO_4XX_SILVER,
];

/// `spectre_v4_safe_list` de `proton-pack.c`.
const SPECTRE_V4_SAFE: [Model; 6] = [
    CORTEX_A35,
    CORTEX_A53,
    CORTEX_A55,
    BRAHMA_B53,
    KRYO_3XX_SILVER,
    KRYO_4XX_SILVER,
];

/// As listas `spectre_bhb_k*_list` (laço de desvios) e `spectre_bhb_firmware_mitigated_list`.
const BHB_AFFECTED: [Model; 16] = [
    (ARM, 0xD41),
    (ARM, 0xD42),
    (ARM, 0xD4B),
    (ARM, 0xD44),
    (ARM, 0xD47),
    (ARM, 0xD48),
    (ARM, 0xD49),
    (ARM, 0xD40),
    (ARM, 0xD0B),
    (ARM, 0xD0D),
    (ARM, 0xD0C),
    (AMPERE, 0xAC3),
    (ARM, 0xD08),
    (ARM, 0xD07),
    (ARM, 0xD09),
    (ARM, 0xD0A),
];

/// Os registradores crus; zero onde a CPU não tem o campo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MitigationRegs {
    pub midr: u64,
    pub id_aa64pfr0: u64,
    pub id_aa64pfr1: u64,
    pub id_aa64mmfr1: u64,
    pub id_aa64isar2: u64,
}

/// Os campos que as mitigações consultam, decodificados.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MitigationFeatures {
    pub implementer: u32,
    /// `MIDR_EL1.Architecture`: `0xF` é "definida pelo ID_*"; o Linux só casa lista com `0xF`.
    pub architecture: u32,
    pub part: u32,
    /// `ID_AA64PFR0_EL1.CSV2`: 1 a 3 dizem que o contexto não envenena o desvio do outro.
    pub csv2: u32,
    /// `ID_AA64PFR0_EL1.CSV3`: diferente de zero, sem Meltdown.
    pub csv3: u32,
    /// `ID_AA64PFR1_EL1.SSBS`: 1 tem `PSTATE.SSBS`; 2 acrescenta `MRS`/`MSR` direto.
    pub ssbs: u32,
    /// `ID_AA64MMFR1_EL1.ECBHB`: a exceção limpa o histórico de desvio.
    pub ecbhb: u32,
    /// `ID_AA64ISAR2_EL1.CLRBHB`: a instrução `CLRBHB` existe.
    pub clrbhb: u32,
}

const fn field(register: u64, shift: u32) -> u32 {
    ((register >> shift) & 0xf) as u32
}

impl MitigationFeatures {
    pub const fn from_regs(regs: MitigationRegs) -> Self {
        Self {
            implementer: ((regs.midr >> 24) & 0xff) as u32,
            architecture: field(regs.midr, 16),
            part: ((regs.midr >> 4) & 0xfff) as u32,
            csv2: field(regs.id_aa64pfr0, 56),
            csv3: field(regs.id_aa64pfr0, 60),
            ssbs: field(regs.id_aa64pfr1, 4),
            ecbhb: field(regs.id_aa64mmfr1, 60),
            clrbhb: field(regs.id_aa64isar2, 28),
        }
    }

    fn in_list(&self, list: &[Model]) -> bool {
        self.architecture == 0xF
            && list
                .iter()
                .any(|&(implementer, part)| implementer == self.implementer && part == self.part)
    }

    /// A CPU consta de alguma lista do Linux; fora delas, não há informação.
    fn known(&self) -> bool {
        [
            &MELTDOWN_SAFE[..],
            &SPECTRE_V2_SAFE,
            &SPECTRE_V4_SAFE,
            &BHB_AFFECTED,
        ]
        .iter()
        .any(|list| self.in_list(list))
    }

    /// As vulnerabilidades pela regra do Linux v6.12 (`cpufeature.c`, `proton-pack.c`). MDS é só
    /// da Intel. CPU fora de todas as listas conta como afetada pelo BHB (o Linux só marca as
    /// listadas); com `ECBHB` ela é afetada e mitigada por hardware.
    pub fn vulnerabilities(&self) -> Vulns {
        let bhb_affected = self.clrbhb != 0 || self.in_list(&BHB_AFFECTED) || !self.known();
        let bhb = if self.csv2 == 3 || !bhb_affected {
            Exposure::NotAffected
        } else if self.ecbhb != 0 {
            Exposure::HardwareMitigated
        } else {
            Exposure::Affected
        };
        Vulns {
            meltdown: self.csv3 == 0 && !self.in_list(&MELTDOWN_SAFE),
            spectre_v2: self.csv2 == 0 && !self.in_list(&SPECTRE_V2_SAFE),
            ssb: !self.in_list(&SPECTRE_V4_SAFE),
            mds: false,
            bhb,
        }
    }
}

impl fmt::Display for MitigationFeatures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "implementer {:#x} part {:#x} csv2 {} csv3 {} ssbs {} ecbhb {} clrbhb {}",
            self.implementer, self.part, self.csv2, self.csv3, self.ssbs, self.ecbhb, self.clrbhb
        )
    }
}

#[cfg(test)]
mod tests;
