//! Compilação do kernel para o alvo bare-metal.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::arch::Arch;
use crate::workspace;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    Dev,
    Release,
    ReleaseSmall,
}

impl Profile {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "dev" => Ok(Self::Dev),
            "release" => Ok(Self::Release),
            "release-small" => Ok(Self::ReleaseSmall),
            other => Err(format!(
                "perfil desconhecido `{other}` (dev, release, release-small)"
            )),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Release => "release",
            Self::ReleaseSmall => "release-small",
        }
    }

    /// Se o perfil leva o modo de teste (feature `selftest`): todos, menos a imagem enxuta.
    pub const fn has_selftest(self) -> bool {
        !matches!(self, Self::ReleaseSmall)
    }

    /// O cargo grava o perfil `dev` em `debug/`; os outros, no próprio nome.
    const fn output_dir(self) -> &'static str {
        match self {
            Self::Dev => "debug",
            other => other.name(),
        }
    }
}

pub fn kernel_path(root: &Path, arch: Arch, profile: Profile) -> PathBuf {
    root.join("target")
        .join(arch.target())
        .join(profile.output_dir())
        .join("rustos")
}

/// A feature do kernel que traz os autotestes, as medidas e o `bench` (SDD 05).
pub const SELFTEST: &str = "selftest";

/// Features do kernel para o perfil: `RUSTOS_FEATURES` (separadas por vírgula: `debug-addrs`,
/// `panic-tests`, `selftest`) mais `selftest` quando o perfil a leva. `RUSTOS_SELFTEST=1|0` troca
/// o padrão do perfil: `1` liga na `release-small`, `0` desliga nos outros.
pub fn features(profile: Profile) -> Vec<String> {
    let listed = listed_features();
    let wanted = selftest_wanted(profile, std::env::var("RUSTOS_SELFTEST").ok().as_deref());
    with_selftest(listed, wanted)
}

/// O que `RUSTOS_FEATURES` lista, sem o que o perfil acrescenta.
pub fn listed_features() -> Vec<String> {
    parse_features(std::env::var("RUSTOS_FEATURES").ok().as_deref())
}

/// O `smoke` (modo de teste) só passa numa imagem com a feature: liga-a em qualquer perfil.
pub fn features_with_selftest(profile: Profile) -> Vec<String> {
    with_selftest(features(profile), true)
}

pub fn has_feature(features: &[String], name: &str) -> bool {
    features.iter().any(|feature| feature == name)
}

fn selftest_wanted(profile: Profile, choice: Option<&str>) -> bool {
    match choice.map(str::trim) {
        Some("1") => true,
        Some("0") => false,
        Some("") | None => profile.has_selftest(),
        Some(other) => {
            eprintln!(
                "xtask: RUSTOS_SELFTEST=`{other}` inválida (1 ou 0); usando o padrão do perfil"
            );
            profile.has_selftest()
        }
    }
}

fn with_selftest(mut features: Vec<String>, wanted: bool) -> Vec<String> {
    if wanted && !has_feature(&features, SELFTEST) {
        features.push(SELFTEST.to_owned());
    }
    features
}

fn parse_features(value: Option<&str>) -> Vec<String> {
    value
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|feature| !feature.is_empty())
        .map(str::to_owned)
        .collect()
}

pub fn build_kernel(root: &Path, arch: Arch, profile: Profile) -> Result<PathBuf, String> {
    build_kernel_with(root, arch, profile, &features(profile))
}

pub fn build_kernel_with(
    root: &Path,
    arch: Arch,
    profile: Profile,
    features: &[String],
) -> Result<PathBuf, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    workspace::run(
        Command::new(cargo)
            .current_dir(root)
            .args([
                "build",
                "--package",
                "rustos-kernel",
                "--target",
                arch.target(),
            ])
            .args(["--profile", profile.name()])
            .args(feature_args(features)),
    )?;
    Ok(kernel_path(root, arch, profile))
}

fn feature_args(features: &[String]) -> Vec<String> {
    if features.is_empty() {
        return Vec::new();
    }
    vec!["--features".to_owned(), features.join(",")]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_come_from_a_comma_list() {
        assert_eq!(parse_features(None).len(), 0);
        assert_eq!(parse_features(Some(" , ")).len(), 0);
        assert_eq!(parse_features(Some("a, b,")), ["a", "b"]);
        assert_eq!(
            feature_args(&["a".to_owned(), "b".to_owned()]),
            ["--features", "a,b"]
        );
        assert_eq!(feature_args(&[]).len(), 0);
    }

    #[test]
    fn only_the_small_release_leaves_out_the_self_test_mode() {
        for profile in [Profile::Dev, Profile::Release] {
            assert!(selftest_wanted(profile, None), "{profile:?}");
            assert!(!selftest_wanted(profile, Some("0")), "{profile:?}");
        }
        assert!(!selftest_wanted(Profile::ReleaseSmall, None));
        assert!(!selftest_wanted(Profile::ReleaseSmall, Some("")));
        assert!(selftest_wanted(Profile::ReleaseSmall, Some("1")));
        assert!(!selftest_wanted(Profile::ReleaseSmall, Some("sim")));
    }

    #[test]
    fn the_self_test_feature_is_added_once() {
        let listed = vec!["panic-tests".to_owned()];
        assert_eq!(with_selftest(listed.clone(), false), ["panic-tests"]);
        assert_eq!(with_selftest(listed, true), ["panic-tests", SELFTEST]);
        let twice = with_selftest(vec![SELFTEST.to_owned()], true);
        assert_eq!(twice, [SELFTEST]);
    }

    #[test]
    fn dev_profile_lives_in_debug_dir() {
        let path = kernel_path(Path::new("/w"), Arch::X86_64, Profile::Dev);
        assert_eq!(
            path,
            Path::new("/w/target/x86_64-unknown-none/debug/rustos")
        );
    }

    #[test]
    fn release_small_keeps_its_name() {
        let path = kernel_path(Path::new("/w"), Arch::Aarch64, Profile::ReleaseSmall);
        assert!(path.ends_with("aarch64-unknown-none-softfloat/release-small/rustos"));
    }
}
