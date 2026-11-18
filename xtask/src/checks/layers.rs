//! Regras do hexágono (guidelines 03 e 04): quem depende de quem, e onde
//! `unsafe` e `asm!` podem morar.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::{manifest, workspace};

/// Crates de fora do workspace aprovados (guidelines 03, regra 7). Vazio de propósito.
const ALLOWED_EXTERNAL: &[&str] = &[];

/// Os crates de `core/` que o `user/` pode ligar, além do `core/hal` (guidelines 03, regra 11).
const USER_MAY_USE_CORE: &[&str] = &["core/elf", "core/cpio"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Layer {
    Ports,
    Core,
    Lib,
    Adapter,
    /// `arch/<isa>-user`: adaptador do lado do usuário, que o `kernel/` nunca liga (regra 12).
    UserAdapter,
    Kernel,
    /// `user/`: runtime e programas de usuário (regra 11).
    User,
    Tool,
}

impl Layer {
    fn of(member: &str) -> Option<Self> {
        let top = member.split('/').next()?;
        match (top, member) {
            (_, "core/hal") => Some(Self::Ports),
            ("core", _) => Some(Self::Core),
            ("lib", _) => Some(Self::Lib),
            ("arch", _) if member.ends_with("-user") => Some(Self::UserAdapter),
            ("arch" | "boot" | "drivers", _) => Some(Self::Adapter),
            ("kernel", _) => Some(Self::Kernel),
            ("user", _) => Some(Self::User),
            ("xtask", _) => Some(Self::Tool),
            _ => None,
        }
    }

    /// `other_path` é a pasta do alvo: o `user/` só depende dos crates de `core/` que a regra 11
    /// lista.
    fn may_depend_on(self, other: Self, other_path: &str) -> bool {
        use Layer::{Adapter, Core, Lib, Ports, User, UserAdapter};
        match self {
            Self::Ports | Self::Tool => false,
            Self::Core => other == Ports,
            Self::Lib => other == Lib,
            Self::Adapter | Self::UserAdapter => matches!(other, Ports | Lib),
            Self::Kernel => matches!(other, Ports | Core | Lib | Adapter),
            Self::User => match other {
                Ports | UserAdapter | User => true,
                Core => USER_MAY_USE_CORE.contains(&other_path),
                _ => false,
            },
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
                "{path}: pasta fora das camadas (core, lib, arch, boot, drivers, kernel, user, xtask)"
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
                Some(target) if !member.layer.may_depend_on(target.layer, &target.path) => {
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
    if member.layer == Layer::User
        && !member.path.ends_with("/rt")
        && !crate_text.contains("#![forbid(unsafe_code)]")
    {
        violations.push(format!(
            "{}: programa de usuário sem `#![forbid(unsafe_code)]` (só o `user/rt` tem `unsafe`, guidelines 03, regra 11)",
            member.path
        ));
    }
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
    fn the_kernel_never_links_an_arch_user_adapter() {
        let user_adapter = Layer::of("arch/x86_64-user").unwrap();
        assert!(!Layer::Kernel.may_depend_on(user_adapter, "arch/x86_64-user"));
        assert_ne!(user_adapter, Layer::of("arch/x86_64").unwrap());
        assert_eq!(Layer::of("arch/aarch64-user"), Some(Layer::UserAdapter));
        let arch = Layer::of("arch/aarch64").unwrap();
        assert!(!arch.may_depend_on(user_adapter, "arch/x86_64-user"));
        assert!(user_adapter.may_depend_on(Layer::Ports, "core/hal"));
        assert!(!user_adapter.may_depend_on(Layer::Core, "core/mm"));
        assert!(!user_adapter.may_depend_on(Layer::Adapter, "arch/x86_64"));
    }

    #[test]
    fn user_depends_on_the_abi_the_loaders_and_its_own_adapters_only() {
        let user = Layer::of("user/rt").unwrap();
        assert_eq!(user, Layer::User);
        assert!(user.may_depend_on(Layer::Ports, "core/hal"));
        assert!(user.may_depend_on(Layer::Core, "core/elf"));
        assert!(user.may_depend_on(Layer::Core, "core/cpio"));
        assert!(user.may_depend_on(Layer::UserAdapter, "arch/x86_64-user"));
        assert!(user.may_depend_on(Layer::User, "user/rt"));
        assert!(!user.may_depend_on(Layer::Core, "core/mm"));
        assert!(!user.may_depend_on(Layer::Kernel, "kernel"));
        assert!(!user.may_depend_on(Layer::Adapter, "arch/x86_64"));
        assert!(!user.may_depend_on(Layer::Lib, "lib/sync"));
    }

    #[test]
    fn the_real_user_crates_are_classified_and_the_kernel_links_none_of_them() {
        let root = workspace::root();
        let members = manifest::workspace_members(&read(&root.join("Cargo.toml")).unwrap());
        let users: Vec<&String> = members.iter().filter(|m| m.starts_with("user/")).collect();
        assert!(users.len() >= 3, "{users:?}");
        assert!(users.iter().all(|m| Layer::of(m) == Some(Layer::User)));
        let adapters: Vec<&String> = members.iter().filter(|m| m.ends_with("-user")).collect();
        assert_eq!(adapters.len(), 2, "{adapters:?}");
        assert!(
            adapters
                .iter()
                .all(|m| Layer::of(m) == Some(Layer::UserAdapter))
        );
        let kernel = manifest::dependencies(&read(&root.join("kernel/Cargo.toml")).unwrap());
        assert!(
            !kernel.iter().any(|name| name.contains("-user")),
            "{kernel:?}"
        );
        assert!(layers(&root).is_ok());
    }

    #[test]
    fn nothing_depends_on_user() {
        for layer in [
            Layer::Ports,
            Layer::Core,
            Layer::Lib,
            Layer::Adapter,
            Layer::UserAdapter,
            Layer::Kernel,
        ] {
            assert!(!layer.may_depend_on(Layer::User, "user/rt"), "{layer:?}");
        }
    }

    #[test]
    fn dependencies_point_inward() {
        assert!(Layer::Adapter.may_depend_on(Layer::Ports, "core/hal"));
        assert!(Layer::Kernel.may_depend_on(Layer::Adapter, "arch/x86_64"));
        assert!(!Layer::Core.may_depend_on(Layer::Adapter, "arch/x86_64"));
        assert!(!Layer::Adapter.may_depend_on(Layer::Adapter, "arch/aarch64"));
        assert!(!Layer::Adapter.may_depend_on(Layer::Core, "core/mm"));
        assert!(!Layer::Ports.may_depend_on(Layer::Lib, "lib/sync"));
        assert!(!Layer::Lib.may_depend_on(Layer::Ports, "core/hal"));
    }
}
