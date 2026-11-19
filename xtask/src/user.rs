//! Compila os programas de usuário (SDD 17, "Como o xtask compila e empacota"): os mesmos alvos do
//! kernel, com flags próprias e um `--target-dir` à parte para não invalidar o cache dele.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::arch::Arch;
use crate::cargo::Profile;
use crate::workspace;

/// Um programa de usuário: o pacote do cargo, o nome do binário (que é o do arquivo no initramfs)
/// e se só entra na imagem com o modo de teste.
#[derive(Clone, Copy, Debug)]
pub struct Program {
    pub package: &'static str,
    pub file: &'static str,
    pub selftest_only: bool,
}

/// Todos os programas de `user/`. O `init` vai em toda imagem; os de teste, só com `selftest`.
pub const PROGRAMS: [Program; 4] = [
    Program {
        package: "rustos-user-init",
        file: "init",
        selftest_only: false,
    },
    Program {
        package: "rustos-user-test-big",
        file: "big",
        selftest_only: true,
    },
    Program {
        package: "rustos-user-test-handles",
        file: "handles",
        selftest_only: true,
    },
    Program {
        package: "rustos-user-test-lazy",
        file: "lazy",
        selftest_only: true,
    },
];

/// Os programas de uma imagem: o `init`, e os de teste se o kernel tem o modo de teste.
pub fn programs_for(selftest: bool) -> Vec<Program> {
    PROGRAMS
        .into_iter()
        .filter(|program| selftest || !program.selftest_only)
        .collect()
}

pub fn target_dir(root: &Path, arch: Arch) -> PathBuf {
    root.join("target").join("user").join(arch.name())
}

pub fn binary_path(root: &Path, arch: Arch, profile: Profile, program: Program) -> PathBuf {
    target_dir(root, arch)
        .join(arch.target())
        .join(profile.output_dir())
        .join(program.file)
}

pub fn linker_script(root: &Path, arch: Arch) -> PathBuf {
    root.join("user")
        .join("link")
        .join(format!("{}.ld", arch.name()))
}

/// A lista inteira de flags do `rustc` dos programas de usuário. As quatro fontes de `rustflags` do
/// cargo são exclusivas: com `CARGO_ENCODED_RUSTFLAGS` o `pie` do `.cargo/config.toml` (o do
/// kernel) desaparece, e o que o usuário também quiser dele se repete aqui.
pub fn rustflags(root: &Path, arch: Arch) -> Vec<String> {
    let mut flags = vec!["-C".to_owned(), "relocation-model=static".to_owned()];
    if arch == Arch::X86_64 {
        // Com `code-model=small` o `core` pré-compilado (modelo `kernel`) não entra no LTO.
        flags.extend(["-C", "code-model=small", "-C", "lto=off"].map(str::to_owned));
    }
    flags.extend([
        "-C".to_owned(),
        "strip=debuginfo".to_owned(),
        "-C".to_owned(),
        format!("link-arg=-T{}", linker_script(root, arch).display()),
        "-C".to_owned(),
        "link-arg=-zmax-page-size=0x1000".to_owned(),
        "-C".to_owned(),
        "link-arg=-znorelro".to_owned(),
    ]);
    flags
}

/// O valor de `CARGO_ENCODED_RUSTFLAGS`: as flags separadas por `\x1f`.
pub fn encoded_rustflags(root: &Path, arch: Arch) -> String {
    rustflags(root, arch).join("\x1f")
}

/// O `cargo build` dos `packages` para `arch`.
pub fn build_command(root: &Path, arch: Arch, profile: Profile, packages: &[&str]) -> Command {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let mut command = Command::new(cargo);
    command
        .current_dir(root)
        .arg("build")
        .args(packages.iter().flat_map(|package| ["--package", package]))
        .args(["--target", arch.target()])
        .args(["--profile", profile.name()])
        .arg("--target-dir")
        .arg(target_dir(root, arch))
        .env("CARGO_ENCODED_RUSTFLAGS", encoded_rustflags(root, arch));
    command
}

/// Compila `programs` e devolve, de cada um, o nome no initramfs e o caminho do binário. **Cada
/// programa numa invocação do cargo**: com vários `--package` juntos o cargo unifica as features do
/// `rt`, e o `init` ligaria o `raw-syscalls` que só `user/tests/*` pode ter (guidelines 03, regra 11).
pub fn build(
    root: &Path,
    arch: Arch,
    profile: Profile,
    programs: &[Program],
) -> Result<Vec<(&'static str, PathBuf)>, String> {
    for program in programs {
        workspace::run(&mut build_command(root, arch, profile, &[program.package]))?;
    }
    Ok(programs
        .iter()
        .map(|&program| (program.file, binary_path(root, arch, profile, program)))
        .collect())
}

/// `cargo xtask user`: compila todos os programas, os de teste inclusive.
pub fn run(root: &Path, arch: Arch, profile: Profile) -> Result<(), String> {
    for (file, path) in build(root, arch, profile, &PROGRAMS)? {
        let size = std::fs::metadata(&path)
            .map_err(|error| format!("ler {}: {error}", path.display()))?
            .len();
        println!(
            "xtask: {file} de {} ({size} bytes) em {}",
            arch.name(),
            workspace::relative(root, &path)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    const ROOT: &str = "/w";

    fn env_of(command: &Command, key: &str) -> Option<String> {
        command
            .get_envs()
            .find(|(name, _)| *name == OsStr::new(key))
            .and_then(|(_, value)| value)
            .map(|value| value.to_string_lossy().into_owned())
    }

    fn args_of(command: &Command) -> Vec<String> {
        command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_x86_64_flags_are_the_whole_list_with_the_small_code_model() {
        let command = build_command(
            Path::new(ROOT),
            Arch::X86_64,
            Profile::Release,
            &["rustos-user-init"],
        );
        let flags = env_of(&command, "CARGO_ENCODED_RUSTFLAGS").unwrap();
        let flags: Vec<&str> = flags.split('\x1f').collect();
        assert_eq!(
            flags,
            [
                "-C",
                "relocation-model=static",
                "-C",
                "code-model=small",
                "-C",
                "lto=off",
                "-C",
                "strip=debuginfo",
                "-C",
                "link-arg=-T/w/user/link/x86_64.ld",
                "-C",
                "link-arg=-zmax-page-size=0x1000",
                "-C",
                "link-arg=-znorelro",
            ]
        );
    }

    #[test]
    fn the_aarch64_flags_have_no_code_model_and_its_own_script() {
        let flags = rustflags(Path::new(ROOT), Arch::Aarch64);
        assert!(
            !flags
                .iter()
                .any(|flag| flag.contains("code-model") || flag.contains("lto"))
        );
        assert!(flags.contains(&"relocation-model=static".to_owned()));
        assert!(flags.contains(&"link-arg=-T/w/user/link/aarch64.ld".to_owned()));
    }

    #[test]
    fn the_kernel_pie_flag_never_reaches_the_user_build() {
        for arch in [Arch::X86_64, Arch::Aarch64] {
            let flags = encoded_rustflags(Path::new(ROOT), arch);
            assert!(!flags.contains("pie") && !flags.contains("pic"), "{flags}");
        }
    }

    #[test]
    fn each_program_is_built_alone_so_the_features_of_the_rt_do_not_unify() {
        for program in PROGRAMS {
            let command = build_command(
                Path::new(ROOT),
                Arch::X86_64,
                Profile::Dev,
                &[program.package],
            );
            let packages = args_of(&command)
                .iter()
                .filter(|arg| arg.starts_with("rustos-user"))
                .count();
            assert_eq!(packages, 1, "{}", program.file);
        }
    }

    #[test]
    fn only_the_test_programs_link_the_raw_syscalls_in_their_manifests() {
        for program in PROGRAMS.iter().filter(|program| !program.selftest_only) {
            let path = workspace::root()
                .join("user")
                .join(program.file)
                .join("Cargo.toml");
            let manifest = std::fs::read_to_string(path).unwrap();
            assert!(!manifest.contains("raw-syscalls"), "{}", program.file);
        }
    }

    #[test]
    fn the_user_build_has_its_own_target_dir_and_target() {
        let command = build_command(
            Path::new(ROOT),
            Arch::Aarch64,
            Profile::ReleaseSmall,
            &["rustos-user-init", "rustos-user-test-big"],
        );
        assert_eq!(
            args_of(&command),
            [
                "build",
                "--package",
                "rustos-user-init",
                "--package",
                "rustos-user-test-big",
                "--target",
                "aarch64-unknown-none-softfloat",
                "--profile",
                "release-small",
                "--target-dir",
                "/w/target/user/aarch64",
            ]
        );
        assert_eq!(env_of(&command, "RUSTFLAGS"), None);
    }

    #[test]
    fn the_binary_lives_under_the_user_target_dir() {
        assert_eq!(
            binary_path(Path::new(ROOT), Arch::X86_64, Profile::Dev, PROGRAMS[0]),
            Path::new("/w/target/user/x86_64/x86_64-unknown-none/debug/init")
        );
    }

    #[test]
    fn only_a_self_test_image_carries_the_test_programs() {
        let plain: Vec<&str> = programs_for(false).iter().map(|p| p.file).collect();
        let tested: Vec<&str> = programs_for(true).iter().map(|p| p.file).collect();
        assert_eq!(plain, ["init"]);
        assert_eq!(tested, ["init", "big", "handles", "lazy"]);
    }

    #[test]
    fn every_program_is_a_workspace_member() {
        let manifest = std::fs::read_to_string(workspace::root().join("Cargo.toml")).unwrap();
        for program in PROGRAMS {
            let dir = if program.selftest_only {
                format!("user/tests/{}", program.file)
            } else {
                format!("user/{}", program.file)
            };
            assert!(manifest.contains(&format!("\"{dir}\"")), "{dir}");
        }
    }
}
