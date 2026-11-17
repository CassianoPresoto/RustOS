//! Regras do hexágono (guidelines 03 e 04): quem depende de quem, e onde
//! `unsafe` e `asm!` podem morar.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::{manifest, workspace};

/// Crates de fora do workspace aprovados (guidelines 03, regra 7). Vazio de propósito.
const ALLOWED_EXTERNAL: &[&str] = &[];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layer {
    Ports,
    Core,
    Lib,
    Adapter,
    Kernel,
    Tool,
}

impl Layer {
    fn of(member: &str) -> Option<Self> {
        let top = member.split('/').next()?;
        match (top, member) {
            (_, "core/hal") => Some(Self::Ports),
            ("core", _) => Some(Self::Core),
            ("lib", _) => Some(Self::Lib),
            ("arch" | "boot" | "drivers", _) => Some(Self::Adapter),
            ("kernel", _) => Some(Self::Kernel),
            ("xtask", _) => Some(Self::Tool),
            _ => None,
        }
    }

    fn may_depend_on(self, other: Self) -> bool {
        use Layer::{Adapter, Core, Lib, Ports};
        match self {
            Self::Ports | Self::Tool => false,
            Self::Core => other == Ports,
            Self::Lib => other == Lib,
            Self::Adapter => matches!(other, Ports | Lib),
            Self::Kernel => matches!(other, Ports | Core | Lib | Adapter),
        }
    }
}

struct Member {
    path: String,
    name: String,
    layer: Layer,
    manifest: String,
}

pub fn layers(root: &Path) -> Result<(), String> {
    let root_manifest = read(&root.join("Cargo.toml"))?;
    let mut violations = Vec::new();
    let mut members = Vec::new();
    for path in manifest::workspace_members(&root_manifest) {
        let Some(layer) = Layer::of(&path) else {
            violations.push(format!(
                "{path}: pasta fora das camadas (core, lib, arch, boot, drivers, kernel, xtask)"
            ));
            continue;
        };
        let text = read(&root.join(&path).join("Cargo.toml"))?;
        let name = manifest::package_name(&text).unwrap_or_else(|| path.clone());
        members.push(Member {
            path,
            name,
            layer,
            manifest: text,
        });
    }
    let by_name: HashMap<&str, &Member> = members
        .iter()
        .map(|member| (member.name.as_str(), member))
        .collect();

    for member in &members {
        for dependency in manifest::dependencies(&member.manifest) {
            match by_name.get(dependency.as_str()) {
                Some(target) if !member.layer.may_depend_on(target.layer) => {
                    violations.push(format!(
                        "{}: {:?} não pode depender de {:?} ({})",
                        member.path, member.layer, target.layer, target.path
                    ));
                }
                Some(_) => {}
                None if ALLOWED_EXTERNAL.contains(&dependency.as_str()) => {}
                None => violations.push(format!(
                    "{}: crate externo `{dependency}` sem aprovação (guidelines 03, regra 7)",
                    member.path
                )),
            }
        }
        check_sources(root, member, &mut violations);
    }
    super::verdict(
        "check-layers",
        &format!("{} crates", members.len()),
        &violations,
    )
}

/// `core/` e `drivers/` proíbem `unsafe`; `asm!` só em `arch/`; crate de kernel é `no_std`.
fn check_sources(root: &Path, member: &Member, violations: &mut Vec<String>) {
    let dir = root.join(&member.path);
    let crate_root = ["src/lib.rs", "src/main.rs"]
        .iter()
        .map(|file| dir.join(file))
        .find(|path| path.is_file());
    let crate_text = crate_root
        .as_deref()
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
    let must_forbid_unsafe =
        matches!(member.layer, Layer::Ports | Layer::Core) || member.path.starts_with("drivers/");
    if must_forbid_unsafe && !crate_text.contains("#![forbid(unsafe_code)]") {
        violations.push(format!(
            "{}: falta `#![forbid(unsafe_code)]` na raiz do crate",
            member.path
        ));
    }
    if member.layer != Layer::Tool && !crate_text.contains("no_std") {
        violations.push(format!("{}: crate do kernel sem `no_std`", member.path));
    }
    let asm_allowed = member.path.starts_with("arch/") || member.layer == Layer::Tool;
    for file in workspace::files_with_extension(&dir.join("src"), "rs") {
        let text = fs::read_to_string(&file).unwrap_or_default();
        if !asm_allowed && (text.contains("asm!(") || text.contains("naked_asm!(")) {
            violations.push(format!(
                "{}: `asm!` fora de arch/",
                workspace::relative(root, &file)
            ));
        }
    }
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("ler {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_members_by_folder() {
        assert_eq!(Layer::of("core/hal"), Some(Layer::Ports));
        assert_eq!(Layer::of("core/mm"), Some(Layer::Core));
        assert_eq!(Layer::of("drivers/pl011"), Some(Layer::Adapter));
        assert_eq!(Layer::of("misc/x"), None);
    }

    #[test]
    fn dependencies_point_inward() {
        assert!(Layer::Adapter.may_depend_on(Layer::Ports));
        assert!(Layer::Kernel.may_depend_on(Layer::Adapter));
        assert!(!Layer::Core.may_depend_on(Layer::Adapter));
        assert!(!Layer::Adapter.may_depend_on(Layer::Adapter));
        assert!(!Layer::Adapter.may_depend_on(Layer::Core));
        assert!(!Layer::Ports.may_depend_on(Layer::Lib));
        assert!(!Layer::Lib.may_depend_on(Layer::Ports));
    }
}
