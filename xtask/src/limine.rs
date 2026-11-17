//! Binários do Limine: versão fixa, baixados uma vez e conferidos por SHA-256 (DEBT-0001).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::workspace;

pub const VERSION: &str = "12.10.0";
const SHA256: &str = "365ba8785d4e4b55c3368738a642681b31f606afec9f77179e07fc26ebe841e2";

fn url() -> String {
    format!(
        "https://github.com/Limine-Bootloader/Limine/releases/download/v{VERSION}/limine-binary.tar.gz"
    )
}

/// Diretório com `BOOTX64.EFI`, `BOOTAA64.EFI` e a licença, baixando se faltar.
pub fn ensure(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join("target").join("limine").join(VERSION);
    let extracted = dir.join("limine-binary");
    if extracted.join("BOOTX64.EFI").is_file() {
        return Ok(extracted);
    }
    std::fs::create_dir_all(&dir).map_err(|error| format!("criar {}: {error}", dir.display()))?;
    let archive = dir.join("limine-binary.tar.gz");
    println!("xtask: baixando Limine {VERSION}");
    workspace::run(
        Command::new("curl")
            .args(["--fail", "--location", "--silent", "--show-error"])
            .arg("--output")
            .arg(&archive)
            .arg(url()),
    )?;
    verify(&archive)?;
    workspace::run(
        Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&dir),
    )?;
    Ok(extracted)
}

fn verify(archive: &Path) -> Result<(), String> {
    let digest = sha256(archive)?;
    if digest == SHA256 {
        return Ok(());
    }
    std::fs::remove_file(archive).ok();
    Err(format!(
        "SHA-256 do Limine não confere: esperado {SHA256}, veio {digest}"
    ))
}

/// `sha256sum` no Linux, `shasum -a 256` no macOS.
fn sha256(path: &Path) -> Result<String, String> {
    let attempts: [&[&str]; 2] = [&["sha256sum"], &["shasum", "-a", "256"]];
    for attempt in attempts {
        let Ok(output) = Command::new(attempt[0])
            .args(&attempt[1..])
            .arg(path)
            .output()
        else {
            continue;
        };
        if output.status.success() {
            let text = String::from_utf8_lossy(&output.stdout);
            return text
                .split_whitespace()
                .next()
                .map(str::to_owned)
                .ok_or_else(|| "saída vazia".to_owned());
        }
    }
    Err("nem `sha256sum` nem `shasum` disponíveis".to_owned())
}
