//! Ferramenta de host do RustOS: compila, monta a imagem, sobe no QEMU, mede e
//! roda os gates de arquitetura. Só `std`: nenhum crate externo.

#![allow(clippy::std_instead_of_core, reason = "ferramenta de host, com std")]

mod arch;
mod cargo;
mod checks;
mod font_preview;
mod image;
mod initramfs;
mod leak;
mod limine;
mod manifest;
mod panic_smoke;
mod png;
mod qemu;
mod size;
mod workspace;

use std::process::ExitCode;

use arch::Arch;
use cargo::Profile;

const USAGE: &str = "\
uso: cargo xtask <comando> [--arch A] [--profile P]

comandos:
  build           compila o kernel
  image           monta a partição EFI (Limine + kernel) em target/esp/<arch>
  run             sobe no QEMU com a serial no terminal (Ctrl-A X sai)
  smoke           sobe no QEMU no modo de teste (`rustos.selftest=1`: autotestes e benchmarks),
                  espera o marcador de boot e sai com o resultado; liga a feature `selftest`
  smoke-fast      o mesmo no modo normal, o boot que o usuário vê; confere o tempo de boot e,
                  na imagem sem a feature `selftest` (release-small), a recusa do modo de teste
  smoke-panic     derruba o kernel de propósito (pânico cedo, com o console preso, aninhado)
                  e confere a mensagem na serial; RUSTOS_FEATURES leva `panic-tests` sozinho
  screenshot      sobe sem janela, digita os comandos depois de `--` (um por argumento
                  ou separados por `;`) e grava a tela em target/screenshots/<arch>.png
  size            tamanho das seções do kernel (o que ocupa memória)
  check-layers    dependências entre camadas e onde `unsafe`/`asm!` podem morar
  check-comments  política de comentários e referências a DEBT-NNNN
  check-docs      índices, frontmatter das dívidas e links de docs/
  check           os três check-*
  initramfs       escreve target/initramfs/<arch>.cpio (newc) com os arquivos dados:
                  `initramfs [--arch A] [--out caminho] [nome=]arquivo...`
  font-preview    gera target/font-preview.png com a grade da fonte (--scale N, 1 a 8)

  A = x86_64 | aarch64 | all  (padrão: x86_64)
  P = dev | release | release-small  (padrão: dev)";

fn main() -> ExitCode {
    match run(&std::env::args().skip(1).collect::<Vec<_>>()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

struct Options {
    arches: Vec<Arch>,
    profile: Profile,
    /// Tudo depois de `--`: os comandos que o `screenshot` digita no shell.
    typed: Vec<String>,
}

fn run(args: &[String]) -> Result<(), String> {
    let Some((command, rest)) = args.split_first() else {
        println!("{USAGE}");
        return Ok(());
    };
    let root = workspace::root();
    if command == "font-preview" {
        return font_preview::run(&root, rest);
    }
    if command == "initramfs" {
        return initramfs::run(&root, rest);
    }
    let options = parse_options(rest)?;
    match command.as_str() {
        "build" => each_arch(&options, |arch| {
            cargo::build_kernel(&root, arch, options.profile).map(drop)
        }),
        "image" => each_arch(&options, |arch| {
            image::assemble(&root, arch, options.profile).map(drop)
        }),
        "run" => each_arch(&options, |arch| qemu::run(&root, arch, options.profile)),
        "smoke" => each_arch(&options, |arch| {
            qemu::smoke(&root, arch, options.profile, qemu::BootMode::SelfTest)
        }),
        "smoke-fast" => each_arch(&options, |arch| {
            qemu::smoke_fast(&root, arch, options.profile)
        }),
        "smoke-panic" => each_arch(&options, |arch| {
            panic_smoke::run(&root, arch, options.profile)
        }),
        "screenshot" => each_arch(&options, |arch| {
            let path = qemu::screenshot(&root, arch, options.profile, &options.typed)?;
            println!("xtask: tela de {} em {}", arch.name(), path.display());
            Ok(())
        }),
        "size" => each_arch(&options, |arch| size::report(&root, arch, options.profile)),
        "check-layers" => checks::layers(&root),
        "check-comments" => checks::comments(&root),
        "check-docs" => checks::docs(&root),
        "check" => checks::all(&root),
        "help" | "--help" | "-h" => {
            println!("{USAGE}");
            Ok(())
        }
        other => Err(format!("comando desconhecido `{other}`\n\n{USAGE}")),
    }
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let (args, typed) = match args.iter().position(|arg| arg == "--") {
        Some(split) => (&args[..split], args[split + 1..].to_vec()),
        None => (args, Vec::new()),
    };
    let mut options = Options {
        arches: vec![Arch::X86_64],
        profile: Profile::Dev,
        typed,
    };
    let mut args = args.iter();
    while let Some(flag) = args.next() {
        let value = args
            .next()
            .ok_or_else(|| format!("`{flag}` precisa de valor"))?;
        match flag.as_str() {
            "--arch" => options.arches = Arch::parse(value)?,
            "--profile" => options.profile = Profile::parse(value)?,
            _ => return Err(format!("opção desconhecida `{flag}`")),
        }
    }
    Ok(options)
}

fn each_arch(
    options: &Options,
    mut task: impl FnMut(Arch) -> Result<(), String>,
) -> Result<(), String> {
    options.arches.iter().try_for_each(|&arch| task(arch))
}
