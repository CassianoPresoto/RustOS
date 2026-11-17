//! Gates de `make check`: falham com a lista de violações, cada uma com endereço.

mod comments;
mod docs;
mod layers;

use std::path::Path;

pub use comments::comments;
pub use docs::docs;
pub use layers::layers;

pub fn all(root: &Path) -> Result<(), String> {
    let results = [
        ("check-layers", layers(root)),
        ("check-comments", comments(root)),
        ("check-docs", docs(root)),
    ];
    let failures: Vec<String> = results
        .into_iter()
        .filter_map(|(name, result)| result.err().map(|error| format!("{name}: {error}")))
        .collect();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// Imprime o resultado de um gate e o converte em erro se houver violação.
fn verdict(name: &str, checked: &str, violations: &[String]) -> Result<(), String> {
    if violations.is_empty() {
        println!("{name}: ok ({checked})");
        return Ok(());
    }
    for violation in violations {
        eprintln!("  {violation}");
    }
    Err(format!("{} violação(ões)", violations.len()))
}
