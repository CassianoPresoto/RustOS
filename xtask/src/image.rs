//! Partição EFI em diretório: a QEMU a expõe como disco FAT (vvfat), sem gerar ISO.

use std::fs;
use std::path::{Path, PathBuf};

use crate::arch::Arch;
use crate::cargo::{self, Profile};
use crate::limine;

fn limine_conf(arch: Arch, cmdline: Option<&str>) -> String {
    let cmdline = cmdline.map_or_else(String::new, |line| format!("    cmdline: {line}\n"));
    format!(
        "timeout: 0\nserial: yes\n\n/RustOS\n    protocol: limine\n    path: boot():/boot/rustos\n    kaslr: yes\n    resolution: {}\n{cmdline}",
        arch.screen_resolution()
    )
}

pub fn esp_dir(root: &Path, arch: Arch) -> PathBuf {
    root.join("target").join("esp").join(arch.name())
}

/// `EFI/BOOT/<loader>` e `limine.conf` ao lado dele, onde o Limine procura primeiro.
pub fn assemble(root: &Path, arch: Arch, profile: Profile) -> Result<PathBuf, String> {
    assemble_with(root, arch, profile, &cargo::features(), None)
}

/// Como `assemble`, com as features do kernel e a linha de comando da entrada do Limine.
pub fn assemble_with(
    root: &Path,
    arch: Arch,
    profile: Profile,
    features: &[String],
    cmdline: Option<&str>,
) -> Result<PathBuf, String> {
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
    fn the_command_line_goes_into_the_boot_entry_only_when_given() {
        assert!(!limine_conf(Arch::X86_64, None).contains("cmdline"));
        let conf = limine_conf(Arch::Aarch64, Some("rustos.panic_early=1"));
        assert!(conf.ends_with("    cmdline: rustos.panic_early=1\n"));
        assert!(conf.contains("kaslr: yes"));
    }
}
