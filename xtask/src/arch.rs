//! O que muda por ISA alvo: triple, loader EFI, QEMU e firmware.

use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Arch {
    pub fn parse(value: &str) -> Result<Vec<Self>, String> {
        match value {
            "x86_64" => Ok(vec![Self::X86_64]),
            "aarch64" => Ok(vec![Self::Aarch64]),
            "all" => Ok(vec![Self::X86_64, Self::Aarch64]),
            other => Err(format!(
                "arquitetura desconhecida `{other}` (x86_64, aarch64, all)"
            )),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }

    pub const fn target(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64-unknown-none",
            Self::Aarch64 => "aarch64-unknown-none-softfloat",
        }
    }

    pub const fn efi_loader(self) -> &'static str {
        match self {
            Self::X86_64 => "BOOTX64.EFI",
            Self::Aarch64 => "BOOTAA64.EFI",
        }
    }

    /// Modo de vídeo pedido ao Limine. Pedir 1280x800 ao GOP do ramfb deixa a
    /// tela em 800x600; 1024x768 ele aceita.
    pub const fn screen_resolution(self) -> &'static str {
        match self {
            Self::X86_64 => "1280x800",
            Self::Aarch64 => "1024x768",
        }
    }

    pub const fn qemu(self) -> &'static str {
        match self {
            Self::X86_64 => "qemu-system-x86_64",
            Self::Aarch64 => "qemu-system-aarch64",
        }
    }

    const fn firmware_env(self) -> &'static str {
        match self {
            Self::X86_64 => "RUSTOS_FIRMWARE_X86_64",
            Self::Aarch64 => "RUSTOS_FIRMWARE_AARCH64",
        }
    }

    /// Onde Homebrew, Debian/Ubuntu e Fedora/Arch instalam o EDK2 da QEMU.
    const fn firmware_candidates(self) -> &'static [&'static str] {
        match self {
            Self::X86_64 => &[
                "/opt/homebrew/share/qemu/edk2-x86_64-code.fd",
                "/usr/local/share/qemu/edk2-x86_64-code.fd",
                "/usr/share/qemu/edk2-x86_64-code.fd",
                "/usr/share/OVMF/OVMF_CODE_4M.fd",
                "/usr/share/OVMF/OVMF_CODE.fd",
                "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
                "/usr/share/edk2/ovmf/OVMF_CODE.fd",
            ],
            Self::Aarch64 => &[
                "/opt/homebrew/share/qemu/edk2-aarch64-code.fd",
                "/usr/local/share/qemu/edk2-aarch64-code.fd",
                "/usr/share/qemu/edk2-aarch64-code.fd",
                "/usr/share/qemu-efi-aarch64/QEMU_EFI.fd",
                "/usr/share/AAVMF/AAVMF_CODE.fd",
                "/usr/share/edk2/aarch64/QEMU_EFI.fd",
            ],
        }
    }

    pub fn firmware(self) -> Result<PathBuf, String> {
        if let Ok(path) = std::env::var(self.firmware_env()) {
            return Ok(PathBuf::from(path));
        }
        self.firmware_candidates()
            .iter()
            .map(PathBuf::from)
            .find(|path| path.is_file())
            .ok_or_else(|| {
                format!(
                    "firmware UEFI de {} não encontrado; instale a QEMU com EDK2 ou defina {}",
                    self.name(),
                    self.firmware_env()
                )
            })
    }

    /// Máquina, CPU, firmware e aceleração; o disco e a serial ficam com quem chama.
    pub fn qemu_machine_args(self, firmware: &str, esp: &str) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        let mut push = |items: &[&str]| args.extend(items.iter().map(|&item| item.to_owned()));
        match self {
            Self::X86_64 => {
                push(&["-machine", "q35", "-cpu", "max", "-m", "256M"]);
                push(&[
                    "-drive",
                    &format!("if=pflash,format=raw,readonly=on,file={firmware}"),
                ]);
                push(&["-drive", &format!("format=raw,file=fat:rw:{esp}")]);
            }
            Self::Aarch64 => {
                push(&[
                    "-machine", "virt", "-cpu", "max", "-m", "256M", "-bios", firmware,
                ]);
                push(&[
                    "-drive",
                    &format!("if=none,id=esp,format=raw,file=fat:rw:{esp}"),
                ]);
                push(&["-device", "virtio-blk-pci,drive=esp"]);
                // A `virt` não tem placa de vídeo; o ramfb vira o GOP do EDK2.
                push(&["-device", "ramfb"]);
            }
        }
        // Sem a placa de rede padrão: a da `virt` pede o efi-virtio.rom do iPXE,
        // que o Ubuntu só instala com o pacote ipxe-qemu.
        push(&["-nic", "none"]);
        for accel in self.accelerators() {
            push(&["-accel", accel]);
        }
        args
    }

    /// Aceleração de hardware quando host e alvo coincidem; TCG sempre como reserva.
    fn accelerators(self) -> Vec<&'static str> {
        let same_isa = std::env::consts::ARCH == self.name();
        let native = match std::env::consts::OS {
            "macos" if same_isa => Some("hvf"),
            "linux" if same_isa && std::path::Path::new("/dev/kvm").exists() => Some("kvm"),
            _ => None,
        };
        native.into_iter().chain(["tcg"]).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_into_both_arches() {
        assert_eq!(Arch::parse("all").unwrap(), [Arch::X86_64, Arch::Aarch64]);
        assert!(Arch::parse("riscv64").is_err());
    }

    #[test]
    fn tcg_is_always_the_last_fallback() {
        for arch in [Arch::X86_64, Arch::Aarch64] {
            assert_eq!(arch.accelerators().last(), Some(&"tcg"));
        }
    }
}
