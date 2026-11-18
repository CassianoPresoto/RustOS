//! Partição EFI em diretório: a QEMU a expõe como disco FAT (vvfat), sem gerar ISO.

use std::fs;
use std::path::{Path, PathBuf};

use crate::arch::Arch;
use crate::cargo::{self, Profile};
use crate::{initramfs, limine, user};

/// Onde o initramfs fica na ESP e o `module_string` que o kernel procura (`vm.rs`).
const INITRAMFS_PATH: &str = "boot/initramfs.cpio";
const INITRAMFS_STRING: &str = "initramfs";

fn limine_conf(arch: Arch, cmdline: Option<&str>) -> String {
    let cmdline = cmdline.map_or_else(String::new, |line| format!("    cmdline: {line}\n"));
    format!(
        "timeout: 0\nserial: yes\n\n/RustOS\n    protocol: limine\n    path: boot():/boot/rustos\n    kaslr: yes\n    resolution: {}\n    module_path: boot():/{INITRAMFS_PATH}\n    module_string: {INITRAMFS_STRING}\n{cmdline}",
        arch.screen_resolution()
    )
}

pub fn esp_dir(root: &Path, arch: Arch) -> PathBuf {
    root.join("target").join("esp").join(arch.name())
}

/// `EFI/BOOT/<loader>` e `limine.conf` ao lado dele, onde o Limine procura primeiro.
pub fn assemble(root: &Path, arch: Arch, profile: Profile) -> Result<PathBuf, String> {
    assemble_with(root, arch, profile, &cargo::features(profile), None)
}

/// Como `assemble`, com as features do kernel e a linha de comando da entrada do Limine.
pub fn assemble_with(
    root: &Path,
    arch: Arch,
    profile: Profile,
    features: &[String],
    cmdline: Option<&str>,
) -> Result<PathBuf, String> {
    let selftest = cargo::has_feature(features, cargo::SELFTEST);
    let programs = user::build(root, arch, profile, &user::programs_for(selftest))?;
    let kernel = cargo::build_kernel_with(root, arch, profile, features)?;
    let limine = limine::ensure(root)?;
    let esp = esp_dir(root, arch);
    let boot_dir = esp.join("EFI").join("BOOT");
    let kernel_dir = esp.join("boot");
    for dir in [&boot_dir, &kernel_dir] {
        fs::create_dir_all(dir).map_err(|error| format!("criar {}: {error}", dir.display()))?;
    }
    copy(
        &limine.join(arch.efi_loader()),
        &boot_dir.join(arch.efi_loader()),
    )?;
    copy(&kernel, &kernel_dir.join("rustos"))?;
    copy(
        &initramfs::write_image(root, arch, &programs)?,
        &esp.join(INITRAMFS_PATH),
    )?;
    fs::write(boot_dir.join("limine.conf"), limine_conf(arch, cmdline))
        .map_err(|error| format!("gravar limine.conf: {error}"))?;
    Ok(esp)
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to)
        .map(drop)
        .map_err(|error| format!("copiar {} para {}: {error}", from.display(), to.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_initramfs_is_a_module_with_the_string_the_kernel_looks_for() {
        let conf = limine_conf(Arch::X86_64, None);
        assert!(conf.contains(
            "    module_path: boot():/boot/initramfs.cpio\n    module_string: initramfs\n"
        ));
        assert!(!conf.contains("module_cmdline"));
    }

    #[test]
    fn the_command_line_goes_into_the_boot_entry_only_when_given() {
        assert!(!limine_conf(Arch::X86_64, None).contains("cmdline"));
        let conf = limine_conf(Arch::Aarch64, Some("rustos.panic_early=1"));
        assert!(conf.ends_with("    cmdline: rustos.panic_early=1\n"));
        assert!(conf.contains("kaslr: yes"));
    }
}
