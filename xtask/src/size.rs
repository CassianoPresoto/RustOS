//! Quanto o kernel ocupa, por seção do ELF: a primeira métrica de recurso.

use std::path::Path;

use crate::arch::Arch;
use crate::cargo::{self, Profile};

const SHF_WRITE: u64 = 0x1;
const SHF_ALLOC: u64 = 0x2;
const SHF_EXECINSTR: u64 = 0x4;
const SHT_NOBITS: u32 = 8;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Footprint {
    pub text: u64,
    pub rodata: u64,
    pub data: u64,
    pub bss: u64,
}

impl Footprint {
    pub const fn loaded(&self) -> u64 {
        self.text + self.rodata + self.data + self.bss
    }
}

pub fn report(root: &Path, arch: Arch, profile: Profile) -> Result<(), String> {
    let kernel = cargo::build_kernel(root, arch, profile)?;
    let bytes =
        std::fs::read(&kernel).map_err(|error| format!("ler {}: {error}", kernel.display()))?;
    let footprint = footprint(&bytes)?;
    println!(
        "kernel {} ({}): {}",
        arch.name(),
        profile.name(),
        kernel.display()
    );
    for (name, size) in [
        ("text", footprint.text),
        ("rodata", footprint.rodata),
        ("data", footprint.data),
        ("bss", footprint.bss),
        ("carregado", footprint.loaded()),
        ("arquivo", bytes.len() as u64),
    ] {
        let tenths = size % 1024 * 10 / 1024;
        println!(
            "  {name:<10} {size:>10} bytes  {:>6}.{tenths} KiB",
            size / 1024
        );
    }
    Ok(())
}

/// Soma as seções alocadas de um ELF64 little-endian por tipo de conteúdo.
pub fn footprint(elf: &[u8]) -> Result<Footprint, String> {
    if elf.get(..4) != Some(b"\x7fELF") || elf.get(4) != Some(&2) || elf.get(5) != Some(&1) {
        return Err("não é um ELF64 little-endian".to_owned());
    }
    let section_offset = to_usize(read_u64(elf, 0x28)?)?;
    let entry_size = usize::from(read_u16(elf, 0x3A)?);
    let count = usize::from(read_u16(elf, 0x3C)?);
    let mut footprint = Footprint::default();
    for index in 0..count {
        let header = section_offset + index * entry_size;
        let kind = read_u32(elf, header + 0x04)?;
        let flags = read_u64(elf, header + 0x08)?;
        let size = read_u64(elf, header + 0x20)?;
        if flags & SHF_ALLOC == 0 {
            continue;
        }
        let bucket = if kind == SHT_NOBITS {
            &mut footprint.bss
        } else if flags & SHF_EXECINSTR != 0 {
            &mut footprint.text
        } else if flags & SHF_WRITE != 0 {
            &mut footprint.data
        } else {
            &mut footprint.rodata
        };
        *bucket += size;
    }
    Ok(footprint)
}

fn field<const N: usize>(elf: &[u8], at: usize) -> Result<[u8; N], String> {
    elf.get(at..at + N)
        .and_then(|slice| slice.try_into().ok())
        .ok_or_else(|| format!("ELF truncado no byte {at}"))
}

fn read_u16(elf: &[u8], at: usize) -> Result<u16, String> {
    field(elf, at).map(u16::from_le_bytes)
}

fn read_u32(elf: &[u8], at: usize) -> Result<u32, String> {
    field(elf, at).map(u32::from_le_bytes)
}

fn read_u64(elf: &[u8], at: usize) -> Result<u64, String> {
    field(elf, at).map(u64::from_le_bytes)
}

fn to_usize(value: u64) -> Result<usize, String> {
    usize::try_from(value).map_err(|_| "deslocamento não cabe em usize".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ELF mínimo: cabeçalho de 64 bytes e uma tabela de seções logo depois.
    fn elf(sections: &[(u32, u64, u64)]) -> Vec<u8> {
        let mut bytes = vec![0u8; 64];
        bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
        bytes[0x28..0x30].copy_from_slice(&64u64.to_le_bytes());
        bytes[0x3A..0x3C].copy_from_slice(&64u16.to_le_bytes());
        bytes[0x3C..0x3E].copy_from_slice(&u16::try_from(sections.len()).unwrap().to_le_bytes());
        for &(kind, flags, size) in sections {
            let mut header = vec![0u8; 64];
            header[0x04..0x08].copy_from_slice(&kind.to_le_bytes());
            header[0x08..0x10].copy_from_slice(&flags.to_le_bytes());
            header[0x20..0x28].copy_from_slice(&size.to_le_bytes());
            bytes.extend(header);
        }
        bytes
    }

    #[test]
    fn buckets_sections_by_flags() {
        let image = elf(&[
            (1, SHF_ALLOC | SHF_EXECINSTR, 100),
            (1, SHF_ALLOC, 20),
            (1, SHF_ALLOC | SHF_WRITE, 8),
            (SHT_NOBITS, SHF_ALLOC | SHF_WRITE, 4096),
            (1, 0, 999),
        ]);
        let footprint = footprint(&image).unwrap();
        assert_eq!(
            footprint,
            Footprint {
                text: 100,
                rodata: 20,
                data: 8,
                bss: 4096
            }
        );
        assert_eq!(footprint.loaded(), 4224);
    }

    #[test]
    fn rejects_non_elf_input() {
        assert!(footprint(b"MZ\x90\x00").is_err());
    }

    #[test]
    fn rejects_truncated_section_table() {
        let mut image = elf(&[(1, SHF_ALLOC, 1)]);
        image.truncate(80);
        assert!(footprint(&image).is_err());
    }
}
