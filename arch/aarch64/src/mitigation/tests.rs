//! Registradores de identificação dos modelos de CPU da QEMU 10.0.0 (`target/arm/tcg/cpu64.c`,
//! etiqueta `v10.0.0`), que copiam os dos núcleos reais; cada teste diz o modelo. Os valores
//! sintéticos dizem que são.

use super::*;

fn regs(midr: u64, pfr0: u64, pfr1: u64, mmfr1: u64) -> MitigationRegs {
    MitigationRegs {
        midr,
        id_aa64pfr0: pfr0,
        id_aa64pfr1: pfr1,
        id_aa64mmfr1: mmfr1,
        id_aa64isar2: 0,
    }
}

fn vulns([meltdown, spectre_v2, ssb, bhb]: [bool; 4]) -> Vulns {
    Vulns {
        meltdown,
        spectre_v2,
        ssb,
        mds: false,
        bhb: if bhb {
            Exposure::Affected
        } else {
            Exposure::NotAffected
        },
    }
}

/// `-cpu cortex-a55`: `aarch64_a55_initfn`.
fn cortex_a55() -> MitigationFeatures {
    MitigationFeatures::from_regs(regs(
        0x412F_D050,
        0x0000_0000_1011_2222,
        0x10,
        0x0000_0000_1021_2122,
    ))
}

/// `-cpu neoverse-n1`: `aarch64_neoverse_n1_initfn`.
fn neoverse_n1() -> MitigationFeatures {
    MitigationFeatures::from_regs(regs(
        0x414F_D0C1,
        0x1100_0000_1011_1112,
        0x20,
        0x0000_0000_1021_2122,
    ))
}

/// `-cpu a64fx`: `aarch64_a64fx_initfn`.
fn a64fx() -> MitigationFeatures {
    MitigationFeatures::from_regs(regs(
        0x461F_0010,
        0x0000_0001_0111_1111,
        0,
        0x0000_0000_1121_2100,
    ))
}

#[test]
fn the_fields_come_from_their_bits() {
    let features = MitigationFeatures::from_regs(MitigationRegs {
        midr: 0x0000_0000_410F_D0C1,
        id_aa64pfr0: 0x3 << 56 | 0x1 << 60,
        id_aa64pfr1: 0x2 << 4,
        id_aa64mmfr1: 0x1 << 60,
        id_aa64isar2: 0x1 << 28,
    });
    assert_eq!(
        features,
        MitigationFeatures {
            implementer: 0x41,
            architecture: 0xF,
            part: 0xD0C,
            csv2: 3,
            csv3: 1,
            ssbs: 2,
            ecbhb: 1,
            clrbhb: 1,
        }
    );
}

#[test]
fn a_neighbouring_field_does_not_leak_into_the_wanted_one() {
    let features = MitigationFeatures::from_regs(MitigationRegs {
        midr: 0,
        id_aa64pfr0: !(0xF << 56) & !(0xF << 60),
        id_aa64pfr1: !(0xF << 4),
        id_aa64mmfr1: !(0xF << 60),
        id_aa64isar2: !(0xF << 28),
    });
    assert_eq!((features.csv2, features.csv3, features.ssbs), (0, 0, 0));
    assert_eq!((features.ecbhb, features.clrbhb), (0, 0));
}

#[test]
fn the_cortex_a55_is_on_every_safe_list() {
    let features = cortex_a55();
    assert_eq!((features.implementer, features.part), (0x41, 0xD05));
    assert_eq!((features.csv2, features.csv3, features.ssbs), (0, 0, 1));
    assert_eq!(
        features.vulnerabilities(),
        vulns([false, false, false, false])
    );
}

#[test]
fn the_cortex_a35_is_on_every_safe_list() {
    // `-cpu cortex-a35`, `aarch64_a35_initfn`.
    let features = MitigationFeatures::from_regs(regs(0x411F_D040, 0x2222, 0, 0));
    assert_eq!(
        features.vulnerabilities(),
        vulns([false, false, false, false])
    );
}

#[test]
fn the_cortex_a72_is_safe_from_meltdown_only_by_the_list() {
    // `-cpu cortex-a72`, `aarch64_a72_initfn`.
    let features = MitigationFeatures::from_regs(regs(0x410F_D083, 0x2222, 0, 0));
    assert_eq!((features.csv2, features.csv3), (0, 0));
    assert_eq!(features.vulnerabilities(), vulns([false, true, true, true]));
}

#[test]
fn the_neoverse_n1_has_csv2_and_csv3_but_still_needs_the_bhb_loop() {
    let features = neoverse_n1();
    assert_eq!((features.csv2, features.csv3, features.ssbs), (1, 1, 2));
    assert_eq!(
        features.vulnerabilities(),
        vulns([false, false, true, true])
    );
}

#[test]
fn the_cortex_a76_and_the_neoverse_v1_read_the_same_way() {
    // `-cpu cortex-a76` e `-cpu neoverse-v1`.
    let a76 = MitigationFeatures::from_regs(regs(
        0x414F_D0B1,
        0x1100_0000_1011_1112,
        0x10,
        0x0000_0000_1021_2122,
    ));
    let v1 = MitigationFeatures::from_regs(regs(
        0x411F_D402,
        0x1101_1101_2011_1112,
        0x20,
        0x0000_0000_1021_2122,
    ));
    assert_eq!(a76.vulnerabilities(), vulns([false, false, true, true]));
    assert_eq!(v1.vulnerabilities(), vulns([false, false, true, true]));
}

#[test]
fn the_neoverse_n2_and_the_cortex_a710_have_csv2_2_and_ssbs2() {
    // `-cpu neoverse-n2` e `-cpu cortex-a710`.
    let n2 = MitigationFeatures::from_regs(regs(
        0x410F_D493,
        0x1201_1111_2011_1112,
        0x221,
        0x0000_0000_1021_2122,
    ));
    let a710 = MitigationFeatures::from_regs(regs(
        0x412F_D471,
        0x1201_1111_2011_1112,
        0x221,
        0x0000_0000_1021_2122,
    ));
    assert_eq!((n2.csv2, n2.csv3, n2.ssbs, n2.ecbhb), (2, 1, 2, 0));
    assert_eq!(n2.vulnerabilities(), vulns([false, false, true, true]));
    assert_eq!(a710.vulnerabilities(), vulns([false, false, true, true]));
}

#[test]
fn the_a64fx_is_in_no_list_and_has_no_csv_field() {
    let features = a64fx();
    assert_eq!((features.implementer, features.part), (0x46, 0x001));
    assert_eq!((features.csv2, features.csv3, features.ssbs), (0, 0, 0));
    assert_eq!(features.vulnerabilities(), vulns([true, true, true, true]));
}

#[test]
fn csv2_3_means_no_bhb_even_on_a_listed_cpu() {
    // Sintético: o Neoverse-N1 com `CSV2 = 3`, como o `-cpu max`.
    let features = MitigationFeatures::from_regs(regs(0x414F_D0C1, 0x3 << 56 | 0x1 << 60, 0x20, 0));
    assert_eq!(features.vulnerabilities().bhb, Exposure::NotAffected);
}

#[test]
fn the_qemu_max_cpu_is_only_exposed_to_ssb() {
    // Sintético, do `aarch64_max_tcg_initfn`: `MIDR` com implementador 0 e peça `Q`, `CSV2 = 3`,
    // `CSV3 = 1`, `SSBS = 2`.
    let features =
        MitigationFeatures::from_regs(regs(0x000F_0510, 0x3 << 56 | 0x1 << 60, 0x2 << 4, 0));
    assert_eq!(
        features.vulnerabilities(),
        vulns([false, false, true, false])
    );
}

#[test]
fn ecbhb_makes_the_cpu_affected_and_mitigated_in_hardware() {
    // Sintético: o A64FX com `ECBHB = 1`; o Linux a trata como afetada e mitigada.
    let features =
        MitigationFeatures::from_regs(regs(0x461F_0010, 0x0000_0001_0111_1111, 0, 0x1 << 60));
    assert_eq!(features.ecbhb, 1);
    assert_eq!(features.vulnerabilities().bhb, Exposure::HardwareMitigated);
}

#[test]
fn clrbhb_makes_the_cpu_affected_but_with_an_instruction() {
    // Sintético: o Cortex-A55 com `CLRBHB = 1` em `ID_AA64ISAR2_EL1`.
    let features = MitigationFeatures::from_regs(MitigationRegs {
        id_aa64isar2: 0x1 << 28,
        ..regs(0x412F_D050, 0x0000_0000_1011_2222, 0x10, 0)
    });
    assert_eq!(features.clrbhb, 1);
    assert_eq!(features.vulnerabilities().bhb, Exposure::Affected);
}

#[test]
fn a_list_match_needs_architecture_0xf() {
    // Sintético: o Cortex-A53 com `Architecture = 8`: o Linux não casa a lista.
    let features = MitigationFeatures::from_regs(regs(0x4108_D034, 0, 0, 0));
    assert_eq!(features.architecture, 8);
    assert_eq!(features.vulnerabilities(), vulns([true, true, true, true]));
    let features = MitigationFeatures::from_regs(regs(0x410F_D034, 0, 0, 0));
    assert_eq!(
        features.vulnerabilities(),
        vulns([false, false, false, false])
    );
}

#[test]
fn the_variant_and_revision_do_not_matter() {
    for midr in [0x410F_D030, 0x413F_D03F, 0x41FF_D03A] {
        let features = MitigationFeatures::from_regs(regs(midr, 0, 0, 0));
        assert!(!features.vulnerabilities().meltdown, "{midr:#x}");
    }
}

#[test]
fn another_implementer_with_the_same_part_is_not_the_listed_cpu() {
    // Sintético: peça `0xD03` com o implementador da Fujitsu.
    let features = MitigationFeatures::from_regs(regs(0x460F_D030, 0, 0, 0));
    assert!(features.vulnerabilities().meltdown);
}

#[test]
fn no_register_information_means_vulnerable_except_mds() {
    let features = MitigationFeatures::from_regs(MitigationRegs::default());
    assert_eq!(features.vulnerabilities(), vulns([true, true, true, true]));
}

#[test]
fn the_non_arm_safe_cpus_are_listed_by_implementer_and_part() {
    // Sintético, das listas do Linux: ThunderX2, Brahma-B53, Carmel e Kryo 4XX silver.
    for (implementer, part) in [
        (0x43_u32, 0x0AF_u32),
        (0x42, 0x100),
        (0x4E, 0x004),
        (0x51, 0x805),
    ] {
        let midr = u64::from(implementer) << 24 | 0xF << 16 | u64::from(part) << 4;
        let features = MitigationFeatures::from_regs(regs(midr, 0, 0, 0));
        assert!(
            !features.vulnerabilities().meltdown,
            "{implementer:#x}/{part:#x}"
        );
    }
}

#[test]
fn the_display_lists_the_fields() {
    assert_eq!(
        neoverse_n1().to_string(),
        "implementer 0x41 part 0xd0c csv2 1 csv3 1 ssbs 2 ecbhb 0 clrbhb 0"
    );
}
