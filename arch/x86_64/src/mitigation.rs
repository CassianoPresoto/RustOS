//! Recursos de CPU das mitigações (CPUID e `IA32_ARCH_CAPABILITIES`) e a regra do Linux para
//! decidir quais vulnerabilidades a CPU tem. Puro: a leitura dos registradores está em `sysreg`.

use core::fmt;

use rustos_hal::mitigation::{Exposure, Vulns};

/// `CPUID.1:ECX`, `CPUID.7.0:EBX`, `CPUID.7.0:EDX`, `CPUID.80000008H:EBX` e
/// `CPUID.80000021H:EAX` (`cpufeatures.h` do Linux, palavras 4, 9, 18, 13 e 20).
const L1_ECX_PCID: u32 = 1 << 17;
const L1_ECX_HYPERVISOR: u32 = 1 << 31;
const L7_EBX_INVPCID: u32 = 1 << 10;
const L7_EDX_MD_CLEAR: u32 = 1 << 10;
const L7_EDX_SPEC_CTRL: u32 = 1 << 26;
const L7_EDX_STIBP: u32 = 1 << 27;
const L7_EDX_ARCH_CAPABILITIES: u32 = 1 << 29;
const L7_EDX_SSBD: u32 = 1 << 31;
const E8_EBX_AMD_IBPB: u32 = 1 << 12;
const E8_EBX_AMD_IBRS: u32 = 1 << 14;
const E8_EBX_AMD_STIBP: u32 = 1 << 15;
const E8_EBX_AMD_SSBD: u32 = 1 << 24;
const E8_EBX_VIRT_SSBD: u32 = 1 << 25;
const E8_EBX_AMD_SSB_NO: u32 = 1 << 26;
const E21_EAX_AUTOIBRS: u32 = 1 << 8;

/// O MSR `IA32_ARCH_CAPABILITIES`, que o `CPUID.7.0:EDX[29]` anuncia.
pub const MSR_ARCH_CAPABILITIES: u32 = 0x10A;

/// As folhas cruas; as de uma folha que a CPU não tem ficam zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MitigationLeaves {
    pub max_basic: u32,
    /// `EBX`, `EDX` e `ECX` da folha 0, a ordem em que formam o texto do fabricante.
    pub vendor: [u32; 3],
    pub leaf1_eax: u32,
    pub leaf1_ecx: u32,
    pub leaf7_ebx: u32,
    pub leaf7_edx: u32,
    pub max_ext: u32,
    pub ext8_ebx: u32,
    pub ext21_eax: u32,
}

/// O fabricante, como o Linux o separa nas tabelas de `cpu/common.c`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Vendor {
    Intel,
    Amd,
    Hygon,
    Centaur,
    Zhaoxin,
    #[default]
    Other,
}

impl Vendor {
    /// `c_ident` de `intel.c`, `amd.c`, `hygon.c`, `centaur.c` e `zhaoxin.c`.
    pub const fn from_leaf0(ebx_edx_ecx: [u32; 3]) -> Self {
        const fn word(text: [u8; 4]) -> u32 {
            u32::from_le_bytes(text)
        }
        match ebx_edx_ecx {
            [a, b, c] if a == word(*b"Genu") && b == word(*b"ineI") && c == word(*b"ntel") => {
                Self::Intel
            }
            [a, b, c] if a == word(*b"Auth") && b == word(*b"enti") && c == word(*b"cAMD") => {
                Self::Amd
            }
            [a, b, c] if a == word(*b"Hygo") && b == word(*b"nGen") && c == word(*b"uine") => {
                Self::Hygon
            }
            [a, b, c] if a == word(*b"Cent") && b == word(*b"aurH") && c == word(*b"auls") => {
                Self::Centaur
            }
            [a, b, c] if a == word(*b"  Sh") && b == word(*b"angh") && c == word(*b"ai  ") => {
                Self::Zhaoxin
            }
            _ => Self::Other,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::Intel => "intel",
            Self::Amd => "amd",
            Self::Hygon => "hygon",
            Self::Centaur => "centaur",
            Self::Zhaoxin => "zhaoxin",
            Self::Other => "other",
        }
    }
}

/// `IA32_ARCH_CAPABILITIES` (`msr-index.h` do Linux, v6.12). Zero quando a CPU não o anuncia.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArchCapabilities(pub u64);

impl ArchCapabilities {
    const fn bit(self, bit: u32) -> bool {
        self.0 & (1 << bit) != 0
    }

    /// Não tem Meltdown.
    pub const fn rdcl_no(self) -> bool {
        self.bit(0)
    }
    /// IBRS melhorado (eIBRS).
    pub const fn ibrs_all(self) -> bool {
        self.bit(1)
    }
    /// `RET` pode usar outros preditores.
    pub const fn rsba(self) -> bool {
        self.bit(2)
    }
    pub const fn ssb_no(self) -> bool {
        self.bit(4)
    }
    pub const fn mds_no(self) -> bool {
        self.bit(5)
    }
    pub const fn taa_no(self) -> bool {
        self.bit(8)
    }
    /// Os três `*_NO` do MMIO obsoleto juntos (`arch_cap_mmio_immune`).
    pub const fn mmio_immune(self) -> bool {
        self.bit(13) && self.bit(14) && self.bit(15)
    }
    /// O `VERW` limpa o buffer de preenchimento mesmo com `MDS_NO`.
    pub const fn fb_clear(self) -> bool {
        self.bit(17)
    }
    pub const fn rrsba(self) -> bool {
        self.bit(19)
    }
    pub const fn bhi_no(self) -> bool {
        self.bit(20)
    }
    pub const fn pbrsb_no(self) -> bool {
        self.bit(24)
    }
    pub const fn gds_no(self) -> bool {
        self.bit(26)
    }
    pub const fn rfds_no(self) -> bool {
        self.bit(27)
    }
}

/// O que o CPUID e o `IA32_ARCH_CAPABILITIES` dizem, decodificado.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "um bit de CPUID por campo")]
pub struct MitigationFeatures {
    pub vendor: Vendor,
    /// Família e modelo como o Linux os monta (`x86_family`, `x86_model`); zero sem a folha 1.
    pub family: u32,
    pub model: u32,
    pub hypervisor: bool,
    pub pcid: bool,
    pub invpcid: bool,
    /// `VERW` limpa os buffers (`MD_CLEAR`).
    pub md_clear: bool,
    /// `IBRS` e `IBPB` pelo `IA32_SPEC_CTRL` e `IA32_PRED_CMD` (Intel).
    pub spec_ctrl: bool,
    pub stibp: bool,
    pub ssbd: bool,
    /// O CPUID anuncia o MSR `IA32_ARCH_CAPABILITIES`.
    pub arch_capabilities: bool,
    pub amd_ibpb: bool,
    pub amd_ibrs: bool,
    pub amd_stibp: bool,
    pub amd_ssbd: bool,
    pub virt_ssbd: bool,
    pub amd_ssb_no: bool,
    /// `AutoIBRS` da AMD, o eIBRS dela.
    pub autoibrs: bool,
    pub arch_cap: ArchCapabilities,
}

const fn family_of(eax: u32) -> u32 {
    let base = (eax >> 8) & 0xf;
    if base == 0xf {
        base + ((eax >> 20) & 0xff)
    } else {
        base
    }
}

const fn model_of(eax: u32) -> u32 {
    let model = (eax >> 4) & 0xf;
    if family_of(eax) >= 6 {
        model + (((eax >> 16) & 0xf) << 4)
    } else {
        model
    }
}

impl MitigationFeatures {
    pub const fn from_leaves(raw: MitigationLeaves) -> Self {
        const fn gate(present: bool, value: u32) -> u32 {
            if present { value } else { 0 }
        }
        let has_leaf1 = raw.max_basic >= 1;
        let has_leaf7 = raw.max_basic >= 7;
        let has_ext8 = raw.max_ext >= 0x8000_0008;
        let has_ext21 = raw.max_ext >= 0x8000_0021;
        let leaves = MitigationLeaves {
            leaf1_eax: gate(has_leaf1, raw.leaf1_eax),
            leaf1_ecx: gate(has_leaf1, raw.leaf1_ecx),
            leaf7_ebx: gate(has_leaf7, raw.leaf7_ebx),
            leaf7_edx: gate(has_leaf7, raw.leaf7_edx),
            ext8_ebx: gate(has_ext8, raw.ext8_ebx),
            ext21_eax: gate(has_ext21, raw.ext21_eax),
            ..raw
        };
        Self {
            vendor: Vendor::from_leaf0(leaves.vendor),
            family: if has_leaf1 {
                family_of(leaves.leaf1_eax)
            } else {
                0
            },
            model: if has_leaf1 {
                model_of(leaves.leaf1_eax)
            } else {
                0
            },
            hypervisor: leaves.leaf1_ecx & L1_ECX_HYPERVISOR != 0,
            pcid: leaves.leaf1_ecx & L1_ECX_PCID != 0,
            invpcid: leaves.leaf7_ebx & L7_EBX_INVPCID != 0,
            md_clear: leaves.leaf7_edx & L7_EDX_MD_CLEAR != 0,
            spec_ctrl: leaves.leaf7_edx & L7_EDX_SPEC_CTRL != 0,
            stibp: leaves.leaf7_edx & L7_EDX_STIBP != 0,
            ssbd: leaves.leaf7_edx & L7_EDX_SSBD != 0,
            arch_capabilities: leaves.leaf7_edx & L7_EDX_ARCH_CAPABILITIES != 0,
            amd_ibpb: leaves.ext8_ebx & E8_EBX_AMD_IBPB != 0,
            amd_ibrs: leaves.ext8_ebx & E8_EBX_AMD_IBRS != 0,
            amd_stibp: leaves.ext8_ebx & E8_EBX_AMD_STIBP != 0,
            amd_ssbd: leaves.ext8_ebx & E8_EBX_AMD_SSBD != 0,
            virt_ssbd: leaves.ext8_ebx & E8_EBX_VIRT_SSBD != 0,
            amd_ssb_no: leaves.ext8_ebx & E8_EBX_AMD_SSB_NO != 0,
            autoibrs: leaves.ext21_eax & E21_EAX_AUTOIBRS != 0,
            arch_cap: ArchCapabilities(0),
        }
    }

    /// Guarda o valor lido do MSR, que só vale se o CPUID o anunciou.
    #[must_use]
    pub const fn with_arch_capabilities(mut self, msr: u64) -> Self {
        if self.arch_capabilities {
            self.arch_cap = ArchCapabilities(msr);
        }
        self
    }

    /// eIBRS: `IBRS_ALL` do MSR ou `AutoIBRS` (`cpu_set_bug_bits`).
    pub const fn enhanced_ibrs(&self) -> bool {
        self.arch_cap.ibrs_all() || self.autoibrs
    }

    /// As vulnerabilidades pela regra de `cpu_set_bug_bits` do Linux v6.12. Sem a folha 1 não há
    /// família nem modelo, e a resposta é `Vulns::ALL`.
    pub fn vulnerabilities(&self) -> Vulns {
        if self.family == 0 {
            return Vulns::ALL;
        }
        let listed = whitelist(self.vendor, self.family, self.model);
        if listed & NO_SPECULATION != 0 {
            return Vulns::NONE;
        }
        let cap = self.arch_cap;
        Vulns {
            meltdown: listed & NO_MELTDOWN == 0 && !cap.rdcl_no(),
            spectre_v2: listed & NO_SPECTRE_V2 == 0,
            ssb: listed & NO_SSB == 0 && !cap.ssb_no() && !self.amd_ssb_no,
            mds: listed & NO_MDS == 0 && !cap.mds_no(),
            bhb: if !cap.bhi_no()
                && listed & NO_BHI == 0
                && (self.enhanced_ibrs() || self.hypervisor)
            {
                Exposure::Affected
            } else {
                Exposure::NotAffected
            },
        }
    }
}

/// Os bits da lista de CPUs sem a vulnerabilidade (`NO_*` de `cpu_vuln_whitelist`).
const NO_SPECULATION: u8 = 1 << 0;
const NO_MELTDOWN: u8 = 1 << 1;
const NO_SSB: u8 = 1 << 2;
const NO_MDS: u8 = 1 << 3;
const NO_SPECTRE_V2: u8 = 1 << 4;
const NO_BHI: u8 = 1 << 5;

/// O que a `cpu_vuln_whitelist` do Linux v6.12 diz da CPU, só nos bits que o RustOS usa; a
/// primeira linha que casa vale, como no `x86_match_cpu`. CPU fora da lista não ganha nada
/// (DEBT-0106).
const fn whitelist(vendor: Vendor, family: u32, model: u32) -> u8 {
    match (vendor, family, model) {
        (_, 4, _)
        | (Vendor::Centaur | Vendor::Intel, 5, _)
        | (Vendor::Intel, 6, 0x1C | 0x26 | 0x27 | 0x35 | 0x36) => NO_SPECULATION,
        (Vendor::Intel, 6, 0x37 | 0x4D | 0x4A | 0x4C | 0x57 | 0x85 | 0x0E | 0x5A | 0x75) => NO_SSB,
        (Vendor::Intel, 6, 0x5C | 0x5F | 0x7A) => NO_MDS,
        (Vendor::Amd, 0x0F..=0x12, _) => NO_MELTDOWN | NO_SSB | NO_MDS | NO_BHI,
        (Vendor::Amd | Vendor::Hygon, _, _) => NO_MELTDOWN | NO_MDS | NO_BHI,
        (Vendor::Centaur | Vendor::Zhaoxin, 7, _) => NO_SPECTRE_V2 | NO_BHI,
        _ => 0,
    }
}

impl fmt::Display for MitigationFeatures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} family {} model {:#x}",
            self.vendor.name(),
            self.family,
            self.model
        )?;
        let flags = [
            ("hypervisor", self.hypervisor),
            ("pcid", self.pcid),
            ("invpcid", self.invpcid),
            ("md_clear", self.md_clear),
            ("spec_ctrl", self.spec_ctrl),
            ("stibp", self.stibp),
            ("ssbd", self.ssbd),
            ("amd_ibpb", self.amd_ibpb),
            ("amd_ibrs", self.amd_ibrs),
            ("amd_stibp", self.amd_stibp),
            ("amd_ssbd", self.amd_ssbd),
            ("virt_ssbd", self.virt_ssbd),
            ("amd_ssb_no", self.amd_ssb_no),
            ("autoibrs", self.autoibrs),
        ];
        for (name, present) in flags {
            if present {
                write!(f, " {name}")?;
            }
        }
        if self.arch_capabilities {
            write!(f, " arch_cap={:#x}", self.arch_cap.0)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
