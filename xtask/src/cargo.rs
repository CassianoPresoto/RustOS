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

/// `RUSTOS_FEATURES`: features do kernel separadas por vírgula (`debug-addrs`, `panic-tests`).
pub fn features() -> Vec<String> {
    parse_features(std::env::var("RUSTOS_FEATURES").ok().as_deref())
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
    build_kernel_with(root, arch, profile, &features())
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
