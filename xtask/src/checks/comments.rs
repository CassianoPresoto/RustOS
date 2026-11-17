//! Política de comentários (guidelines 06): no máximo 4 linhas por bloco, e todo
//! `DEBT-NNNN` citado precisa existir em `docs/tech-debt/`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::workspace;

const MAX_BLOCK_LINES: usize = 4;

pub fn comments(root: &Path) -> Result<(), String> {
    let debts = existing_debts(&root.join("docs/tech-debt"));
    let mut violations = Vec::new();
    let mut files = 0;
    for file in workspace::files_with_extension(root, "rs") {
        let text = fs::read_to_string(&file).unwrap_or_default();
        let name = workspace::relative(root, &file);
        files += 1;
        for (line, length) in long_blocks(&text) {
            violations.push(format!(
                "{name}:{line}: bloco de {length} linhas de comentário (máximo {MAX_BLOCK_LINES})"
            ));
        }
        for (line, debt) in debt_references(&text) {
            if !debts.contains(&debt) {
                violations.push(format!(
                    "{name}:{line}: DEBT-{debt} não existe em docs/tech-debt/"
                ));
            }
        }
    }
    super::verdict(
        "check-comments",
        &format!("{files} arquivos .rs"),
        &violations,
    )
}

/// Números das dívidas com arquivo próprio (`NNNN-slug.md`).
pub fn existing_debts(dir: &Path) -> BTreeSet<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return BTreeSet::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let number = name.get(..4)?;
            (number.bytes().all(|b| b.is_ascii_digit()) && is_markdown(&name))
                .then(|| number.to_owned())
        })
        .collect()
}

pub fn is_markdown(name: &str) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

/// (linha inicial, tamanho) de cada bloco contíguo de `//` maior que o limite.
fn long_blocks(text: &str) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    let mut start = 0;
    let mut length = 0;
    for (index, line) in text.lines().chain([""]).enumerate() {
        if line.trim_start().starts_with("//") {
            if length == 0 {
                start = index + 1;
            }
            length += 1;
            continue;
        }
        if length > MAX_BLOCK_LINES {
            found.push((start, length));
        }
        length = 0;
    }
    found
}

/// (linha, número) de cada `DEBT-` seguido de quatro dígitos.
pub fn debt_references(text: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (index, line) in text.lines().enumerate() {
        for (at, _) in line.match_indices("DEBT-") {
            let digits = &line[at + 5..];
            if digits.len() >= 4 && digits.as_bytes()[..4].iter().all(u8::is_ascii_digit) {
                found.push((index + 1, digits[..4].to_owned()));
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: [(usize, usize); 0] = [];

    #[test]
    fn accepts_blocks_up_to_the_limit() {
        assert_eq!(long_blocks("// 1\n// 2\n// 3\n// 4\nfn x() {}\n"), NONE);
    }

    #[test]
    fn flags_long_blocks_including_doc_comments() {
        let text = "fn a() {}\n/// 1\n/// 2\n//! 3\n// 4\n// 5\n";
        assert_eq!(long_blocks(text), [(2, 5)]);
    }

    #[test]
    fn code_with_trailing_comment_breaks_a_block() {
        assert_eq!(
            long_blocks("// 1\n// 2\n// 3\nlet x = 1; // 4\n// 5\n// 6\n"),
            NONE
        );
    }

    #[test]
    fn finds_debt_references_with_four_digits_only() {
        let text = format!(
            "// ver {d}0001\n// {d}XXXX e {d}12 não contam\nx // {d}0042.",
            d = "DEBT-"
        );
        assert_eq!(
            debt_references(&text),
            [(1, "0001".to_owned()), (3, "0042".to_owned())]
        );
    }
}
