//! Recursos da CPU que o CPUID anuncia, decodificados das folhas cruas. Puro.
//! RDRAND é `CPUID.1:ECX[30]` e RDSEED é `CPUID.7.0:EBX[18]` (`cpufeatures.h` do Linux).

/// Bits de `CPUID.1:ECX`, `CPUID.1:EDX`, `CPUID.7.0:EBX`, `CPUID.7.0:ECX` e
/// `CPUID.80000001H:EDX` (SDM vol. 2A, instrução CPUID).
const L1_ECX_X2APIC: u32 = 1 << 21;
const L1_ECX_RDRAND: u32 = 1 << 30;
const L1_EDX_APIC: u32 = 1 << 9;
const L1_EDX_TSC: u32 = 1 << 4;
const L7_EBX_SMEP: u32 = 1 << 7;
const L7_EBX_RDSEED: u32 = 1 << 18;
const L7_EBX_SMAP: u32 = 1 << 20;
const L7_ECX_UMIP: u32 = 1 << 2;
const EXT_EDX_NX: u32 = 1 << 20;

/// As folhas cruas; `leaf7_*` é zero quando a CPU não tem a folha 7.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Leaves {
    pub max_basic: u32,
    pub leaf1_ecx: u32,
    pub leaf1_edx: u32,
    pub leaf7_ebx: u32,
    pub leaf7_ecx: u32,
    pub ext_edx: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools, reason = "um bit de CPUID por campo")]
pub struct Features {
    pub apic: bool,
    pub x2apic: bool,
    pub tsc: bool,
    pub nx: bool,
    pub smep: bool,
    pub smap: bool,
    pub umip: bool,
    pub rdrand: bool,
    pub rdseed: bool,
}

impl Features {
    pub const fn from_leaves(leaves: Leaves) -> Self {
        let has_leaf7 = leaves.max_basic >= 7;
        let (ebx, ecx) = if has_leaf7 {
            (leaves.leaf7_ebx, leaves.leaf7_ecx)
        } else {
            (0, 0)
        };
        Self {
            apic: leaves.leaf1_edx & L1_EDX_APIC != 0,
            x2apic: leaves.leaf1_ecx & L1_ECX_X2APIC != 0,
            tsc: leaves.leaf1_edx & L1_EDX_TSC != 0,
            nx: leaves.ext_edx & EXT_EDX_NX != 0,
            smep: ebx & L7_EBX_SMEP != 0,
            smap: ebx & L7_EBX_SMAP != 0,
            umip: ecx & L7_ECX_UMIP != 0,
            rdrand: leaves.leaf1_ecx & L1_ECX_RDRAND != 0,
            rdseed: ebx & L7_EBX_RDSEED != 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_each_bit_from_its_leaf() {
        let features = Features::from_leaves(Leaves {
            max_basic: 0x16,
            leaf1_ecx: 1 << 21 | 1 << 30,
            leaf1_edx: 1 << 9 | 1 << 4,
            leaf7_ebx: 1 << 7 | 1 << 20 | 1 << 18,
            leaf7_ecx: 1 << 2,
            ext_edx: 1 << 20,
        });
        assert_eq!(
            features,
            Features {
                apic: true,
                x2apic: true,
                tsc: true,
                nx: true,
                smep: true,
                smap: true,
                umip: true,
                rdrand: true,
                rdseed: true,
            }
        );
    }

    #[test]
    fn empty_leaves_announce_nothing() {
        assert_eq!(
            Features::from_leaves(Leaves::default()),
            Features::default()
        );
    }

    #[test]
    fn leaf7_is_ignored_when_the_cpu_does_not_have_it() {
        let features = Features::from_leaves(Leaves {
            max_basic: 6,
            leaf7_ebx: u32::MAX,
            leaf7_ecx: u32::MAX,
            ..Leaves::default()
        });
        assert!(!features.smep && !features.smap && !features.umip && !features.rdseed);
    }

    #[test]
    fn xapic_without_x2apic_is_distinguished() {
        let features = Features::from_leaves(Leaves {
            leaf1_edx: 1 << 9,
            ..Leaves::default()
        });
        assert!(features.apic && !features.x2apic);
    }
}
