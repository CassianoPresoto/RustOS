//! Leitura mínima de `Cargo.toml`: só o que os gates precisam, sem parser TOML externo.

/// Membros do `[workspace]`, na ordem em que aparecem.
pub fn workspace_members(toml: &str) -> Vec<String> {
    let mut members = Vec::new();
    let mut inside = false;
    for line in toml.lines().map(strip_comment) {
        let line = line.trim();
        if inside {
            members.extend(quoted_strings(line));
            if line.contains(']') {
                break;
            }
            continue;
        }
        let Some(rest) = line.strip_prefix("members") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        inside = true;
        members.extend(quoted_strings(rest));
        if rest.contains(']') {
            break;
        }
    }
    members
}

pub fn package_name(toml: &str) -> Option<String> {
    let mut in_package = false;
    for line in toml.lines().map(strip_comment).map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package && let Some(value) = key_value(line, "name") {
            return quoted_strings(value).into_iter().next();
        }
    }
    None
}

/// Nome de toda dependência, de qualquer tipo e alvo: normal, dev, build e `[target.*]`.
pub fn dependencies(toml: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut in_table = false;
    for line in toml.lines().map(strip_comment).map(str::trim) {
        if let Some(header) = line.strip_prefix('[') {
            let header = header.trim_end_matches(']');
            in_table = header.ends_with("dependencies");
            if let Some((_, name)) = header.rsplit_once("dependencies.") {
                found.push(name.trim_matches('"').to_owned());
            }
            continue;
        }
        if in_table && !line.is_empty() {
            let name = line.split(['=', '.', ' ']).next().unwrap_or_default();
            if !name.is_empty() {
                found.push(name.trim_matches('"').to_owned());
            }
        }
    }
    found
}

fn key_value<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?.trim_start();
    rest.strip_prefix('=').map(str::trim)
}

fn strip_comment(line: &str) -> &str {
    line.split_once('#').map_or(line, |(code, _)| code)
}

fn quoted_strings(text: &str) -> Vec<String> {
    text.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_multiline_members_and_ignores_comments() {
        let toml = "[workspace]\nmembers = [\n  \"a\", # comentário\n  \"core/b\",\n]\ndefault-members = [\"x\"]\n";
        assert_eq!(workspace_members(toml), ["a", "core/b"]);
    }

    #[test]
    fn reads_inline_members() {
        assert_eq!(workspace_members("members = [\"a\", \"b\"]"), ["a", "b"]);
    }

    #[test]
    fn reads_package_name_only_from_package_table() {
        let toml = "[lib]\nname = \"other\"\n[package]\nname = \"rustos-mm\"\n";
        assert_eq!(package_name(toml).as_deref(), Some("rustos-mm"));
    }

    #[test]
    fn collects_every_dependency_flavour() {
        let toml = "\
[dependencies]
rustos-hal.workspace = true
serde = { version = \"1\" }

[target.'cfg(target_arch = \"x86_64\")'.dependencies]
rustos-arch-x86_64.workspace = true

[dev-dependencies.proptest]
version = \"1\"

[build-dependencies]
cc = \"1\"

[lints]
workspace = true
";
        assert_eq!(
            dependencies(toml),
            [
                "rustos-hal",
                "serde",
                "rustos-arch-x86_64",
                "proptest",
                "cc"
            ]
        );
    }
}
