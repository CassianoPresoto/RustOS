//! Consistência de `docs/`: índices dos READMEs, frontmatter das dívidas e links relativos.

use std::fs;
use std::path::Path;

use super::comments::{debt_references, existing_debts, is_markdown};
use crate::workspace;

const STATUSES: &[&str] = &["Open", "Paid", "Dropped"];

pub fn docs(root: &Path) -> Result<(), String> {
    let mut violations = Vec::new();
    for folder in ["docs/sdd", "docs/guidelines"] {
        check_index(root, folder, is_chapter, &mut violations);
    }
    check_index(root, "docs/tech-debt", is_debt, &mut violations);
    check_debts(root, &mut violations);
    let markdown = workspace::files_with_extension(root, "md");
    let debts = existing_debts(&root.join("docs/tech-debt"));
    for file in &markdown {
        check_links(root, file, &mut violations);
        let text = fs::read_to_string(file).unwrap_or_default();
        let in_debt_folder = file.starts_with(root.join("docs/tech-debt"));
        for (line, debt) in debt_references(&text) {
            if !debts.contains(&debt) && !in_debt_folder {
                violations.push(format!(
                    "{}:{line}: DEBT-{debt} não existe",
                    workspace::relative(root, file)
                ));
            }
        }
    }
    super::verdict(
        "check-docs",
        &format!("{} arquivos .md", markdown.len()),
        &violations,
    )
}

fn is_chapter(name: &str) -> bool {
    name.len() > 3
        && name.as_bytes()[..2].iter().all(u8::is_ascii_digit)
        && name.as_bytes()[2] == b'_'
}

fn is_debt(name: &str) -> bool {
    name.len() > 5
        && name.as_bytes()[..4].iter().all(u8::is_ascii_digit)
        && name.as_bytes()[4] == b'-'
}

/// Todo arquivo numerado da pasta aparece como link no `README.md` dela.
fn check_index(
    root: &Path,
    folder: &str,
    numbered: fn(&str) -> bool,
    violations: &mut Vec<String>,
) {
    let dir = root.join(folder);
    let readme = fs::read_to_string(dir.join("README.md")).unwrap_or_default();
    let Ok(entries) = fs::read_dir(&dir) else {
        violations.push(format!("{folder}: pasta ausente"));
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_markdown(&name) && numbered(&name) && !readme.contains(&format!("]({name})")) {
            violations.push(format!(
                "{folder}/README.md: falta o link para {name} no índice"
            ));
        }
    }
}

/// Frontmatter das dívidas: `id` igual ao número do arquivo, `status` válido e `trigger` de verdade.
fn check_debts(root: &Path, violations: &mut Vec<String>) {
    let dir = root.join("docs/tech-debt");
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_debt(&name) || !is_markdown(&name) {
            continue;
        }
        let text = fs::read_to_string(entry.path()).unwrap_or_default();
        let field = |key: &str| frontmatter(&text, key);
        let label = format!("docs/tech-debt/{name}");
        if field("id").as_deref() != Some(&format!("DEBT-{}", &name[..4])) {
            violations.push(format!("{label}: `id` precisa ser DEBT-{}", &name[..4]));
        }
        let status = field("status").unwrap_or_default();
        if !STATUSES.contains(&status.as_str()) {
            violations.push(format!(
                "{label}: `status` precisa ser Open, Paid ou Dropped"
            ));
        }
        let trigger = field("trigger").unwrap_or_default();
        if status == "Open" && (trigger.is_empty() || trigger.starts_with("TODO")) {
            violations.push(format!(
                "{label}: dívida aberta sem `trigger` (o evento que obriga a pagar)"
            ));
        }
    }
}

fn frontmatter(text: &str, key: &str) -> Option<String> {
    let body = text.strip_prefix("---\n")?;
    let (header, _) = body.split_once("\n---")?;
    header.lines().find_map(|line| {
        let value = line.strip_prefix(key)?.strip_prefix(':')?;
        let value = value.split_once(" #").map_or(value, |(value, _)| value);
        Some(value.trim().to_owned())
    })
}

/// Links relativos `](caminho)` apontam para arquivo ou pasta que existe.
fn check_links(root: &Path, file: &Path, violations: &mut Vec<String>) {
    let text = fs::read_to_string(file).unwrap_or_default();
    let base = file.parent().unwrap_or(root);
    let mut in_code = false;
    for (index, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
        }
        if in_code {
            continue;
        }
        for target in link_targets(line) {
            let path = target.split('#').next().unwrap_or_default();
            if path.is_empty() || target.contains("://") || target.starts_with("mailto:") {
                continue;
            }
            if !base.join(path).exists() {
                violations.push(format!(
                    "{}:{}: link quebrado `{target}`",
                    workspace::relative(root, file),
                    index + 1
                ));
            }
        }
    }
}

fn link_targets(line: &str) -> Vec<&str> {
    line.match_indices("](")
        .filter_map(|(at, _)| {
            let rest = &line[at + 2..];
            rest.find(')').map(|end| &rest[..end])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_file_naming_conventions() {
        assert!(is_chapter("01_visao.md"));
        assert!(!is_chapter("README.md"));
        assert!(is_debt("0001-boot-por-limine.md"));
        assert!(!is_debt("_template.md"));
    }

    #[test]
    fn reads_frontmatter_ignoring_inline_comments() {
        let text = "---\nid: DEBT-0001\nstatus: Open # Open | Paid\ntrigger: o primeiro boot em hardware real\n---\n# x";
        assert_eq!(frontmatter(text, "id").as_deref(), Some("DEBT-0001"));
        assert_eq!(frontmatter(text, "status").as_deref(), Some("Open"));
        assert_eq!(
            frontmatter(text, "trigger").as_deref(),
            Some("o primeiro boot em hardware real")
        );
        assert_eq!(frontmatter("# sem frontmatter", "id"), None);
    }

    #[test]
    fn extracts_markdown_link_targets() {
        let line = "veja [a](a.md#sec) e [b](https://x.y) e [c](../c/)";
        assert_eq!(link_targets(line), ["a.md#sec", "https://x.y", "../c/"]);
    }
}
