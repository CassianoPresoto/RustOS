//! Flash do EDK2 do aarch64: o código em 64 MiB e a NVRAM de variáveis com `Timeout = 0`.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// A `virt` exige pflash de 64 MiB.
pub const FLASH_SIZE: u64 = 64 << 20;
const BLOCK_BYTES: u32 = 0x4_0000;
const FV_HEADER_BYTES: u16 = 0x48;
const BLOCK: usize = BLOCK_BYTES as usize;
const FV_HEADER_LEN: usize = FV_HEADER_BYTES as usize;
const STORE_HEADER_LEN: usize = 28;
const FIRST_VARIABLE: usize = FV_HEADER_LEN + STORE_HEADER_LEN;

const FV_FILESYSTEM_GUID: [u8; 16] = [
    0x8d, 0x2b, 0xf1, 0xff, 0x96, 0x76, 0x8b, 0x4c, 0xa9, 0x85, 0x27, 0x47, 0x07, 0x5b, 0x4f, 0x50,
];
const AUTH_VARIABLE_STORE_GUID: [u8; 16] = [
    0x78, 0x2c, 0xf3, 0xaa, 0x7b, 0x94, 0x9a, 0x43, 0xa1, 0x80, 0x2e, 0x14, 0x4e, 0xc3, 0x77, 0x92,
];
const GLOBAL_VARIABLE_GUID: [u8; 16] = [
    0x61, 0xdf, 0xe4, 0x8b, 0xca, 0x93, 0xd2, 0x11, 0xaa, 0x0d, 0x00, 0xe0, 0x98, 0x03, 0x2b, 0x8c,
];

/// Imagem do armazém de variáveis: três blocos de 256 KiB (variáveis, FTW de trabalho e reserva)
/// com uma única variável, `Timeout` (UINT16, NV+BS+RT) igual a 0. O resto fica em `0xFF`.
pub fn vars_image() -> Vec<u8> {
    let mut image = vec![0xFF_u8; 3 * BLOCK];
    let header = &mut image[..FV_HEADER_LEN];
    header[..0x10].fill(0);
    header[0x10..0x20].copy_from_slice(&FV_FILESYSTEM_GUID);
    header[0x20..0x28].copy_from_slice(&(3 * u64::from(BLOCK_BYTES)).to_le_bytes());
    header[0x28..0x2c].copy_from_slice(b"_FVH");
    header[0x2c..0x30].copy_from_slice(&0x0004_feff_u32.to_le_bytes());
    header[0x30..0x32].copy_from_slice(&FV_HEADER_BYTES.to_le_bytes());
    header[0x32..0x38].fill(0);
    header[0x37] = 2;
    header[0x38..0x3c].copy_from_slice(&3_u32.to_le_bytes());
    header[0x3c..0x40].copy_from_slice(&BLOCK_BYTES.to_le_bytes());
    header[0x40..].fill(0);
    let sum = word_sum(header);
    header[0x32..0x34].copy_from_slice(&sum.wrapping_neg().to_le_bytes());

    let store = &mut image[FV_HEADER_LEN..FIRST_VARIABLE];
    store.fill(0);
    store[..16].copy_from_slice(&AUTH_VARIABLE_STORE_GUID);
    store[16..20].copy_from_slice(&(BLOCK_BYTES - u32::from(FV_HEADER_BYTES)).to_le_bytes());
    store[20] = 0x5a;
    store[21] = 0xfe;

    let variable = timeout_variable();
    image[FIRST_VARIABLE..FIRST_VARIABLE + variable.len()].copy_from_slice(&variable);
    image
}

/// Soma dos `u16` do cabeçalho; o campo de checksum a leva a zero.
fn word_sum(bytes: &[u8]) -> u16 {
    (0..bytes.len() / 2).fold(0_u16, |sum, i| {
        sum.wrapping_add(u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]))
    })
}

/// `AUTHENTICATED_VARIABLE_HEADER` (60 bytes) + nome em UCS-2 + dado.
fn timeout_variable() -> Vec<u8> {
    let name: Vec<u8> = "Timeout\0"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let mut variable = Vec::new();
    variable.extend(0x55aa_u16.to_le_bytes());
    variable.extend([0x3f, 0]);
    variable.extend(7_u32.to_le_bytes());
    variable.extend([0_u8; 8 + 16 + 4]);
    variable.extend(u32::try_from(name.len()).unwrap_or(0).to_le_bytes());
    variable.extend(2_u32.to_le_bytes());
    variable.extend(GLOBAL_VARIABLE_GUID);
    variable.extend(name);
    variable.extend(0_u16.to_le_bytes());
    variable
}

/// `(código, variáveis)` do pflash do aarch64, em `target/firmware/`. O código sai do EDK2
/// achado, completado até 64 MiB se for menor; as variáveis são sempre refeitas.
pub fn aarch64_flash(root: &Path, firmware: &Path) -> Result<(PathBuf, PathBuf), String> {
    let dir = root.join("target").join("firmware");
    fs::create_dir_all(&dir).map_err(|error| format!("criar {}: {error}", dir.display()))?;
    let source_len = fs::metadata(firmware)
        .map_err(|error| format!("ler {}: {error}", firmware.display()))?
        .len();
    let code = if source_len == FLASH_SIZE {
        firmware.to_path_buf()
    } else {
        let padded = dir.join("aarch64-code.fd");
        let bytes =
            fs::read(firmware).map_err(|error| format!("ler {}: {error}", firmware.display()))?;
        write_flash(&padded, &bytes)?;
        padded
    };
    let vars = dir.join("aarch64-vars.fd");
    write_flash(&vars, &vars_image())?;
    Ok((code, vars))
}

/// Grava `bytes` seguidos de zeros até 64 MiB, por arquivo temporário e `rename`.
fn write_flash(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let temp = path.with_extension(format!("tmp{}", std::process::id()));
    let result = fs::File::create(&temp).and_then(|mut file| {
        file.write_all(bytes)?;
        file.set_len(FLASH_SIZE)?;
        file.sync_all()
    });
    result
        .and_then(|()| fs::rename(&temp, path))
        .map_err(|error| format!("gravar {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_checksum_makes_the_words_sum_to_zero() {
        let image = vars_image();
        assert_eq!(word_sum(&image[..FV_HEADER_LEN]), 0);
        assert_eq!(&image[0x28..0x2c], b"_FVH");
    }

    #[test]
    fn the_image_is_three_blocks_with_the_timeout_first() {
        let image = vars_image();
        assert_eq!(image.len(), 3 * BLOCK);
        assert_eq!(image[FIRST_VARIABLE..FIRST_VARIABLE + 2], [0xaa, 0x55]);
        let variable = timeout_variable();
        assert_eq!(variable.len(), 60 + 16 + 2);
        assert_eq!(
            &image[FIRST_VARIABLE + 4..FIRST_VARIABLE + 8],
            &7_u32.to_le_bytes()
        );
        assert!(
            image[FIRST_VARIABLE + variable.len()..BLOCK]
                .iter()
                .all(|&b| b == 0xFF)
        );
    }

    #[test]
    fn the_flash_file_has_the_size_the_machine_asks_for() {
        let dir = std::env::temp_dir().join(format!("rustos-nvram-{}", std::process::id()));
        let source = dir.join("code.fd");
        fs::create_dir_all(&dir).unwrap();
        fs::write(&source, [1_u8; 4096]).unwrap();
        let (code, vars) = aarch64_flash(&dir, &source).unwrap();
        for path in [code, vars] {
            assert_eq!(fs::metadata(path).unwrap().len(), FLASH_SIZE);
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
