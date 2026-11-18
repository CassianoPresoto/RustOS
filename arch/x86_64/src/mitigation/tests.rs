//! Valores de CPUID e de `IA32_ARCH_CAPABILITIES` de CPUs reais, dos dumps do repositório
//! `InstLatx64/InstLatx64` (commit `17be9a9`, `master` de 2026-10-04): cada teste diz o arquivo.
//! Os valores sintéticos dizem que são.

use super::*;

const INTEL: [u32; 3] = [0x756E_6547, 0x4965_6E69, 0x6C65_746E];
const AMD: [u32; 3] = [0x6874_7541, 0x6974_6E65, 0x444D_4163];
const HYGON: [u32; 3] = [0x6F67_7948, 0x6E65_476E, 0x656E_6975];

fn leaves(
    vendor: [u32; 3],
    leaf1: (u32, u32),
    leaf7: (u32, u32),
    ext: (u32, u32, u32),
) -> MitigationLeaves {
    MitigationLeaves {
        max_basic: 0x16,
        vendor,
        leaf1_eax: leaf1.0,
        leaf1_ecx: leaf1.1,
        leaf7_ebx: leaf7.0,
        leaf7_edx: leaf7.1,
        max_ext: ext.0,
        ext8_ebx: ext.1,
        ext21_eax: ext.2,
    }
}

fn vulns([meltdown, spectre_v2, ssb, mds, bhb]: [bool; 5]) -> Vulns {
    Vulns {
        meltdown,
        spectre_v2,
        ssb,
        mds,
        bhb,
    }
}

/// `GenuineIntel00306C3_Haswell_CPUID.txt`: sem microcode de Spectre, sem o MSR.
fn haswell() -> MitigationFeatures {
    MitigationFeatures::from_leaves(leaves(
        INTEL,
        (0x0003_06C3, 0x7FFA_FBFF),
        (0x0000_27AB, 0),
        (0x8000_0008, 0, 0),
    ))
}

/// `GenuineIntel0050657_CascadeLakeSP_CPUID1.txt`, MSR `0x10A` = `0x2B`.
fn cascade_lake() -> MitigationFeatures {
    MitigationFeatures::from_leaves(leaves(
        INTEL,
        (0x0005_0657, 0x7FFE_FBFF),
        (0xD39F_FFFB, 0xBC00_0400),
        (0x8000_0008, 0, 0),
    ))
    .with_arch_capabilities(0x2B)
}

/// `AuthenticAMD0830F10_K17_Rome_CPUID.txt`: Zen 2, sem PCID.
fn zen2_rome() -> MitigationFeatures {
    MitigationFeatures::from_leaves(leaves(
        AMD,
        (0x0083_0F10, 0x7EF8_320B),
        (0x219C_91A9, 0),
        (0x8000_0020, 0x018E_F757, 0x0000_0000),
    ))
}

#[test]
fn haswell_has_pcid_and_none_of_the_speculation_controls() {
    let features = haswell();
    assert_eq!(
        (features.vendor, features.family, features.model),
        (Vendor::Intel, 6, 0x3C)
    );
    assert!(features.pcid && features.invpcid);
    assert!(!features.hypervisor);
    assert!(!features.spec_ctrl && !features.stibp && !features.ssbd);
    assert!(!features.md_clear && !features.arch_capabilities);
    assert_eq!(features.arch_cap, ArchCapabilities(0));
}

#[test]
fn haswell_is_vulnerable_to_everything_but_branch_history() {
    assert_eq!(
        haswell().vulnerabilities(),
        vulns([true, true, true, true, false])
    );
}

#[test]
fn skylake_without_microcode_looks_like_haswell() {
    // `GenuineIntel00506E3_Skylake_CPUID.txt`: `CPUID.7.0:EDX` = 0.
    let features = MitigationFeatures::from_leaves(leaves(
        INTEL,
        (0x0005_06E3, 0x7FFA_FBFF),
        (0x029C_67AF, 0),
        (0x8000_0008, 0, 0),
    ));
    assert_eq!(features.model, 0x5E);
    assert!(!features.spec_ctrl && !features.arch_capabilities);
    assert_eq!(
        features.vulnerabilities(),
        vulns([true, true, true, true, false])
    );
}

#[test]
fn cascade_lake_announces_the_msr_and_reports_enhanced_ibrs() {
    let features = cascade_lake();
    assert_eq!((features.family, features.model), (6, 0x55));
    assert!(features.spec_ctrl && features.stibp && features.ssbd && features.md_clear);
    assert!(features.arch_capabilities);
    let cap = features.arch_cap;
    assert!(cap.rdcl_no() && cap.ibrs_all() && cap.mds_no());
    assert!(!cap.ssb_no() && !cap.rsba() && !cap.bhi_no() && !cap.taa_no());
    assert!(features.enhanced_ibrs());
}

#[test]
fn cascade_lake_has_no_meltdown_nor_mds_but_keeps_v2_ssb_and_bhi() {
    assert_eq!(
        cascade_lake().vulnerabilities(),
        vulns([false, true, true, false, true])
    );
}

#[test]
fn the_msr_value_is_ignored_when_the_cpuid_does_not_announce_it() {
    let features = haswell().with_arch_capabilities(u64::MAX);
    assert_eq!(features.arch_cap, ArchCapabilities(0));
    assert_eq!(features.vulnerabilities(), haswell().vulnerabilities());
}

#[test]
fn ice_lake_has_the_same_capabilities_as_cascade_lake() {
    // `GenuineIntel00706E5_IceLakeY_CPUID.txt`, MSR `0x10A` = `0x2B`.
    let features = MitigationFeatures::from_leaves(leaves(
        INTEL,
        (0x0007_06E5, 0x7FFA_FBBF),
        (0xF2BF_27EF, 0xBC00_0410),
        (0x8000_0008, 0, 0),
    ))
    .with_arch_capabilities(0x2B);
    assert_eq!((features.family, features.model), (6, 0x7E));
    assert_eq!(
        features.vulnerabilities(),
        vulns([false, true, true, false, true])
    );
}

#[test]
fn sapphire_rapids_still_lacks_bhi_no() {
    // `GenuineIntel00806F8_SapphireRapids_05_CPUID.txt`, MSR `0x10A` = `0x28FDEB`.
    let features = MitigationFeatures::from_leaves(leaves(
        INTEL,
        (0x0008_06F8, 0x7FFE_FBFF),
        (0xF3BF_BFFB, 0xFFDD_4430),
        (0x8000_0008, 0x0000_0200, 0),
    ))
    .with_arch_capabilities(0x0028_FDEB);
    assert_eq!((features.family, features.model), (6, 0x8F));
    let cap = features.arch_cap;
    assert!(cap.rdcl_no() && cap.ibrs_all() && cap.mds_no() && cap.taa_no());
    assert!(cap.mmio_immune() && cap.rrsba());
    assert!(!cap.bhi_no() && !cap.ssb_no() && !cap.gds_no() && !cap.rfds_no());
    assert!(!cap.fb_clear() && !cap.pbrsb_no());
    assert_eq!(
        features.vulnerabilities(),
        vulns([false, true, true, false, true])
    );
}

#[test]
fn alder_lake_reads_the_hybrid_model_and_the_same_verdict() {
    // `GenuineIntel0090672_AlderLake_02_CPUID.txt`, MSR `0x10A` = `0xD6B`.
    let features = MitigationFeatures::from_leaves(leaves(
        INTEL,
        (0x0009_0672, 0x7FFA_FBBF),
        (0xF3BF_A7EB, 0xFC98_4510),
        (0x8000_0008, 0, 0),
    ))
    .with_arch_capabilities(0xD6B);
    assert_eq!((features.family, features.model), (6, 0x97));
    assert!(features.arch_cap.taa_no());
    assert_eq!(
        features.vulnerabilities(),
        vulns([false, true, true, false, true])
    );
}

#[test]
fn goldmont_is_on_the_list_without_mds_and_has_no_msr() {
    // `GenuineIntel00506C9_Goldmont_CPUID.txt`.
    let features = MitigationFeatures::from_leaves(leaves(
        INTEL,
        (0x0005_06C9, 0x4FF8_EBBF),
        (0x2294_E283, 0),
        (0x8000_0008, 0, 0),
    ));
    assert_eq!(features.model, 0x5C);
    assert_eq!(
        features.vulnerabilities(),
        vulns([true, true, true, false, false])
    );
}

#[test]
fn silvermont_is_on_the_list_without_ssb() {
    // `GenuineIntel0030673_Silvermont_CPUID.txt`.
    let features = MitigationFeatures::from_leaves(leaves(
        INTEL,
        (0x0003_0673, 0x43D8_E3BF),
        (0x0000_2282, 0),
        (0x8000_0008, 0, 0),
    ));
    assert_eq!(features.model, 0x37);
    assert_eq!(
        features.vulnerabilities(),
        vulns([true, true, false, true, false])
    );
}

#[test]
fn zen2_has_the_amd_controls_and_no_pcid() {
    let features = zen2_rome();
    assert_eq!(
        (features.vendor, features.family, features.model),
        (Vendor::Amd, 0x17, 0x31)
    );
    assert!(features.amd_ibpb && features.amd_ibrs && features.amd_stibp && features.amd_ssbd);
    assert!(!features.virt_ssbd && !features.amd_ssb_no && !features.autoibrs);
    assert!(!features.pcid && !features.invpcid);
    assert!(!features.spec_ctrl && !features.arch_capabilities);
}

#[test]
fn zen2_is_vulnerable_to_v2_and_ssb_only() {
    assert_eq!(
        zen2_rome().vulnerabilities(),
        vulns([false, true, true, false, false])
    );
}

#[test]
fn zen4_announces_autoibrs_through_the_extended_leaf_21() {
    // `AuthenticAMD0A10F11_K19_Genoa_01_CPUID.txt`.
    let features = MitigationFeatures::from_leaves(leaves(
        AMD,
        (0x00A1_0F11, 0x7EFA_320B),
        (0xF1BF_97A9, 0x1000_0010),
        (0x8000_0028, 0x79BE_F25F, 0x0006_2FCF),
    ));
    assert_eq!((features.family, features.model), (0x19, 0x11));
    assert!(features.autoibrs && features.enhanced_ibrs());
    assert!(features.pcid && features.invpcid && features.amd_ibpb);
    assert_eq!(
        features.vulnerabilities(),
        vulns([false, true, true, false, false])
    );
}

#[test]
fn the_extended_leaf_21_is_ignored_when_the_cpu_stops_before_it() {
    let features = MitigationFeatures::from_leaves(MitigationLeaves {
        max_ext: 0x8000_0020,
        ext21_eax: u32::MAX,
        ..leaves(AMD, (0x0083_0F10, 0), (0, 0), (0x8000_0020, 0, 0))
    });
    assert!(!features.autoibrs);
}

#[test]
fn bulldozer_has_no_leaf7_and_no_amd_controls() {
    // `AuthenticAMD0600F01_K15_Bulldozer_CPUID.txt`: família 0x15.
    let features = MitigationFeatures::from_leaves(MitigationLeaves {
        max_basic: 0xD,
        ..leaves(AMD, (0x0060_0F01, 0x1E98_220B), (0, 0), (0x8000_001E, 0, 0))
    });
    assert_eq!((features.family, features.model), (0x15, 0x00));
    assert!(!features.amd_ibpb && !features.spec_ctrl);
    assert_eq!(
        features.vulnerabilities(),
        vulns([false, true, true, false, false])
    );
}

#[test]
fn hygon_is_listed_like_amd() {
    // `HygonGenuine0900F02_Hygon_CPUID.txt`: família 0x18.
    let features = MitigationFeatures::from_leaves(MitigationLeaves {
        max_basic: 0xD,
        ..leaves(
            HYGON,
            (0x0090_0F02, 0x74D8_3209),
            (0, 0),
            (0x8000_001F, 0x1007, 0),
        )
    });
    assert_eq!((features.vendor, features.family), (Vendor::Hygon, 0x18));
    assert_eq!(
        features.vulnerabilities(),
        vulns([false, true, true, false, false])
    );
}

#[test]
fn the_oldest_amd_families_have_no_ssb_either() {
    // Sintético: família 0x10 (K10), `0x100F42`, sem informação além da identidade.
    let features = MitigationFeatures::from_leaves(MitigationLeaves {
        max_basic: 1,
        ..leaves(AMD, (0x0010_0F42, 0), (0, 0), (0, 0, 0))
    });
    assert_eq!(features.family, 0x10);
    assert_eq!(
        features.vulnerabilities(),
        vulns([false, true, false, false, false])
    );
}

#[test]
fn a_cpu_with_no_speculation_has_nothing_to_mitigate() {
    // Sintético, por `cpu_vuln_whitelist`: Pentium (família 5), 486 (4) e Bonnell (`0x1C`).
    for eax in [0x0000_0543, 0x0000_0480, 0x0001_06C2] {
        let features = MitigationFeatures::from_leaves(MitigationLeaves {
            max_basic: 1,
            ..leaves(INTEL, (eax, 0), (0, 0), (0, 0, 0))
        });
        assert_eq!(features.vulnerabilities(), Vulns::default(), "{eax:#x}");
    }
}

#[test]
fn no_cpuid_information_means_vulnerable_to_everything() {
    let features = MitigationFeatures::from_leaves(MitigationLeaves::default());
    assert_eq!(features.family, 0);
    assert_eq!(features.vulnerabilities(), Vulns::ALL);
}

#[test]
fn leaf1_alone_with_an_unknown_vendor_is_vulnerable_by_default() {
    // Sintético: fabricante que a tabela não lista, família 6.
    let features = MitigationFeatures::from_leaves(MitigationLeaves {
        max_basic: 1,
        ..leaves([0, 0, 0], (0x0000_0663, 0), (0, 0), (0, 0, 0))
    });
    assert_eq!(features.vendor, Vendor::Other);
    assert_eq!(
        features.vulnerabilities(),
        vulns([true, true, true, true, false])
    );
}

#[test]
fn a_hypervisor_makes_bhi_assumed_even_without_enhanced_ibrs() {
    // Sintético: o Haswell com o bit 31 de `CPUID.1:ECX`.
    let features = MitigationFeatures::from_leaves(leaves(
        INTEL,
        (0x0003_06C3, 0x7FFA_FBFF | 1 << 31),
        (0x0000_27AB, 0),
        (0x8000_0008, 0, 0),
    ));
    assert!(features.hypervisor);
    assert!(features.vulnerabilities().bhb);
}

#[test]
fn each_arch_capabilities_bit_flips_its_verdict() {
    // Sintético: um bit por vez sobre o Haswell com o MSR anunciado.
    let announced = |bits: u64| {
        let mut base = leaves(
            INTEL,
            (0x0003_06C3, 0x7FFA_FBFF),
            (0x0000_27AB, 1 << 29),
            (0x8000_0008, 0, 0),
        );
        base.max_basic = 0x16;
        MitigationFeatures::from_leaves(base)
            .with_arch_capabilities(bits)
            .vulnerabilities()
    };
    let base = announced(0);
    assert_eq!(base, vulns([true, true, true, true, false]));
    assert!(!announced(1 << 0).meltdown);
    assert!(!announced(1 << 4).ssb);
    assert!(!announced(1 << 5).mds);
    assert!(announced(1 << 1).bhb);
    assert!(
        !MitigationFeatures::from_leaves(MitigationLeaves {
            leaf7_edx: 1 << 29,
            ..leaves(INTEL, (0x0003_06C3, 0x7FFA_FBFF), (0, 1 << 29), (0, 0, 0))
        })
        .with_arch_capabilities((1 << 1) | (1 << 20))
        .vulnerabilities()
        .bhb
    );
}

#[test]
fn the_amd_ssb_no_bit_clears_ssb() {
    // Sintético: o Rome com `0x80000008:EBX[26]`.
    let features = MitigationFeatures::from_leaves(leaves(
        AMD,
        (0x0083_0F10, 0x7EF8_320B),
        (0x219C_91A9, 0),
        (0x8000_0020, 0x018E_F757 | 1 << 26, 0),
    ));
    assert!(features.amd_ssb_no);
    assert!(!features.vulnerabilities().ssb);
}

#[test]
fn every_cpuid_bit_lands_in_its_field() {
    let features = MitigationFeatures::from_leaves(MitigationLeaves {
        max_basic: 7,
        vendor: INTEL,
        leaf1_eax: 0x0005_06E3,
        leaf1_ecx: 1 << 17 | 1 << 31,
        leaf7_ebx: 1 << 10,
        leaf7_edx: 1 << 10 | 1 << 26 | 1 << 27 | 1 << 29 | 1 << 31,
        max_ext: 0x8000_0021,
        ext8_ebx: 1 << 12 | 1 << 14 | 1 << 15 | 1 << 24 | 1 << 25 | 1 << 26,
        ext21_eax: 1 << 8,
    });
    assert!(features.pcid && features.hypervisor && features.invpcid);
    assert!(features.md_clear && features.spec_ctrl && features.stibp);
    assert!(features.arch_capabilities && features.ssbd);
    assert!(features.amd_ibpb && features.amd_ibrs && features.amd_stibp);
    assert!(features.amd_ssbd && features.virt_ssbd && features.amd_ssb_no);
    assert!(features.autoibrs);
}

#[test]
fn leaf7_is_ignored_when_the_cpu_does_not_have_it() {
    let features = MitigationFeatures::from_leaves(MitigationLeaves {
        max_basic: 6,
        leaf1_eax: 0x0005_06E3,
        leaf7_ebx: u32::MAX,
        leaf7_edx: u32::MAX,
        ..MitigationLeaves::default()
    });
    assert!(!features.invpcid && !features.spec_ctrl && !features.arch_capabilities);
}

#[test]
fn the_vendor_comes_from_the_three_words_in_the_right_order() {
    for (words, vendor) in [
        (INTEL, Vendor::Intel),
        (AMD, Vendor::Amd),
        (HYGON, Vendor::Hygon),
        ([0x746E_6543, 0x4872_7561, 0x736C_7561], Vendor::Centaur),
        ([0x6853_2020, 0x6867_6E61, 0x2020_6961], Vendor::Zhaoxin),
        ([0, 0, 0], Vendor::Other),
        ([INTEL[0], INTEL[2], INTEL[1]], Vendor::Other),
    ] {
        assert_eq!(Vendor::from_leaf0(words), vendor);
    }
}

#[test]
fn family_and_model_follow_the_extended_fields() {
    assert_eq!((family_of(0x0000_0543), model_of(0x0000_0543)), (5, 4));
    assert_eq!((family_of(0x000F_0F41), model_of(0x000F_0F41)), (0xF, 0xF4));
    assert_eq!(
        (family_of(0x0083_0F10), model_of(0x0083_0F10)),
        (0x17, 0x31)
    );
    assert_eq!((family_of(0x0005_06E3), model_of(0x0005_06E3)), (6, 0x5E));
}

#[test]
fn the_display_lists_what_is_present() {
    assert_eq!(
        cascade_lake().to_string(),
        "intel family 6 model 0x55 pcid invpcid md_clear spec_ctrl stibp ssbd arch_cap=0x2b"
    );
    assert_eq!(
        zen2_rome().to_string(),
        "amd family 23 model 0x31 amd_ibpb amd_ibrs amd_stibp amd_ssbd"
    );
}
