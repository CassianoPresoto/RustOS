//! Escritor de cpio `newc` em Rust puro: monta o initramfs que o `core/cpio` lê
//! (SDD 17). Mesma entrada, mesmos bytes; sem o binário `cpio` do host.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use crate::arch::Arch;

const MAGIC: &str = "070701";
const TRAILER: &str = "TRAILER!!!";
const S_IFREG: u32 = 0o100_000;
const MAX_NAME_SIZE: u32 = 4096;
const TRAILER_NAMESIZE: u32 = 11;
const DEFAULT_MODE: u32 = 0o755;

/// Por que uma entrada foi recusada. Cada variante é uma regra do leitor ou da gramática.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// Nome vazio, com NUL, ou com componente vazio, `.` ou `..` (cobre `/` inicial e final).
    BadName(String),
    /// O nome é `TRAILER!!!`, que o leitor trata como o fim do arquivo.
    ReservedName(String),
    /// Nome acima de `MAX_NAME_SIZE - 1` bytes do `core/cpio`.
    NameTooLong(String),
    /// Dois arquivos com o mesmo nome.
    Duplicate(String),
    /// Conteúdo que não cabe em `c_filesize` (8 dígitos hexadecimais).
    TooBig(String),
    /// Mais arquivos do que cabem em `c_ino` (8 dígitos hexadecimais).
    TooMany,
    /// Modo com bit fora das permissões (`0o7777`): o tipo regular é do escritor.
    BadMode(String, u32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadName(name) => write!(f, "nome inválido no initramfs: {name:?}"),
            Self::NameTooLong(name) => write!(f, "nome longo demais no initramfs: {name:?}"),
            Self::Duplicate(name) => write!(f, "nome repetido no initramfs: {name:?}"),
            Self::ReservedName(name) => write!(f, "nome reservado no initramfs: {name:?}"),
            Self::TooMany => write!(f, "arquivos demais para o newc"),
            Self::TooBig(name) => write!(f, "arquivo grande demais para o newc: {name:?}"),
            Self::BadMode(name, mode) => write!(f, "modo {mode:#o} inválido em {name:?}"),
        }
    }
}

/// Um arquivo regular do initramfs: nome, conteúdo e permissões (`0o755`).
pub type File<'a> = (&'a str, &'a [u8], u32);

/// O cpio `newc` com `files` em ordem, fechado pelo `TRAILER!!!`. Não gera
/// diretórios: o leitor do kernel acha o arquivo pelo nome inteiro.
pub fn build(files: &[File<'_>]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    for (index, &(name, data, mode)) in files.iter().enumerate() {
        let namesize = check_name(name)?;
        if files[..index].iter().any(|&(other, ..)| other == name) {
            return Err(Error::Duplicate(name.to_owned()));
        }
        if mode & !0o7777 != 0 {
            return Err(Error::BadMode(name.to_owned(), mode));
        }
        let size = u32::try_from(data.len()).map_err(|_| Error::TooBig(name.to_owned()))?;
        let ino = u32::try_from(index + 1).map_err(|_| Error::TooMany)?;
        entry(
            &mut out,
            (name, namesize),
            data,
            [ino, S_IFREG | mode, size],
        );
    }
    entry(&mut out, (TRAILER, TRAILER_NAMESIZE), &[], [0; 3]);
    Ok(out)
}

/// Valida o nome e devolve o `c_namesize` (com o NUL).
fn check_name(name: &str) -> Result<u32, Error> {
    if name.contains('\0') || name.split('/').any(|part| matches!(part, "" | "." | "..")) {
        return Err(Error::BadName(name.to_owned()));
    }
    if name == TRAILER {
        return Err(Error::ReservedName(name.to_owned()));
    }
    u32::try_from(name.len() + 1)
        .ok()
        .filter(|&size| size <= MAX_NAME_SIZE)
        .ok_or_else(|| Error::NameTooLong(name.to_owned()))
}

/// Cabeçalho, nome com NUL e dados, cada trecho alinhado a 4 a partir do início.
fn entry(
    out: &mut Vec<u8>,
    (name, namesize): (&str, u32),
    data: &[u8],
    [ino, mode, size]: [u32; 3],
) {
    let nlink = 1;
    out.extend_from_slice(MAGIC.as_bytes());
    // ino, mode, uid, gid, nlink, mtime, filesize, dev(2), rdev(2), namesize, check.
    let fields = [ino, mode, 0, 0, nlink, 0, size, 0, 0, 0, 0, namesize, 0];
    for field in fields {
        out.extend_from_slice(format!("{field:08X}").as_bytes());
    }
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    pad4(out);
    out.extend_from_slice(data);
    pad4(out);
}

fn pad4(out: &mut Vec<u8>) {
    out.resize(out.len().next_multiple_of(4), 0);
}

struct Options {
    arches: Vec<Arch>,
    out: Option<PathBuf>,
    inputs: Vec<String>,
}

const USAGE: &str = "uso: initramfs [--arch A] [--out caminho] [nome=]arquivo...\n\
  argumento que existe como arquivo é caminho; senão o primeiro `=` separa o nome (pode ter `/`) do caminho";

fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        arches: vec![Arch::X86_64],
        out: None,
        inputs: Vec::new(),
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--arch" | "--out" => {
                let value = args
                    .next()
                    .ok_or_else(|| format!("`{arg}` precisa de valor"))?;
                if arg == "--arch" {
                    options.arches = Arch::parse(value)?;
                } else {
                    options.out = Some(PathBuf::from(value));
                }
            }
            flag if flag.starts_with("--") => {
                return Err(format!("opção desconhecida `{flag}` ({USAGE})"));
            }
            _ => options.inputs.push(arg.clone()),
        }
    }
    if options.inputs.is_empty() {
        return Err(format!("nenhum arquivo para empacotar ({USAGE})"));
    }
    if options.out.is_some() && options.arches.len() > 1 {
        return Err("`--out` vale para uma ISA só; tire o `all`".to_owned());
    }
    Ok(options)
}

/// Argumento que existe como arquivo é caminho (nome = último componente); senão
/// corta no primeiro `=`: nome antes, caminho depois.
fn split_input(input: &str) -> Result<(String, &str), String> {
    if let Some((name, path)) = input
        .split_once('=')
        .filter(|_| !Path::new(input).is_file())
    {
        return Ok((name.to_owned(), path));
    }
    Path::new(input)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| (name.to_owned(), input))
        .ok_or_else(|| format!("sem nome de arquivo em `{input}`"))
}

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let options = parse(args)?;
    let mut loaded = Vec::new();
    for input in &options.inputs {
        let (name, path) = split_input(input)?;
        let data = fs::read(path).map_err(|error| format!("ler {path}: {error}"))?;
        loaded.push((name, data));
    }
    let files: Vec<File<'_>> = loaded
        .iter()
        .map(|(name, data)| (name.as_str(), data.as_slice(), DEFAULT_MODE))
        .collect();
    let archive = build(&files).map_err(|error| error.to_string())?;
    for &arch in &options.arches {
        let path = options
            .out
            .clone()
            .unwrap_or_else(|| default_path(root, arch));
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(|error| format!("criar {}: {error}", dir.display()))?;
        }
        fs::write(&path, &archive)
            .map_err(|error| format!("gravar {}: {error}", path.display()))?;
        println!(
            "xtask: initramfs de {} ({} arquivos, {} bytes) em {}",
            arch.name(),
            files.len(),
            archive.len(),
            path.display()
        );
    }
    Ok(())
}

pub fn default_path(root: &Path, arch: Arch) -> PathBuf {
    root.join("target")
        .join("initramfs")
        .join(format!("{}.cpio", arch.name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    type Parsed = (String, Vec<u8>, u32);

    /// Lê o `newc` pela gramática, sem o `core/cpio`, até o trailer.
    fn parse_back(archive: &[u8]) -> Vec<Parsed> {
        let field = |at: usize, index: usize| {
            let start = at + 6 + index * 8;
            let text = std::str::from_utf8(&archive[start..start + 8]).unwrap();
            u32::from_str_radix(text, 16).unwrap()
        };
        let (mut at, mut found) = (0, Vec::new());
        loop {
            assert_eq!(&archive[at..at + 6], b"070701");
            let (mode, size, namesize) = (field(at, 1), field(at, 6), field(at, 11) as usize);
            let size = size as usize;
            let name_at = at + 110;
            assert_eq!(archive[name_at + namesize - 1], 0, "nome sem NUL");
            let name = String::from_utf8(archive[name_at..name_at + namesize - 1].to_vec());
            let data_at = (name_at + namesize).next_multiple_of(4);
            let data = archive[data_at..data_at + size].to_vec();
            at = (data_at + size).next_multiple_of(4);
            let name = name.unwrap();
            if name == "TRAILER!!!" {
                assert_eq!(at, archive.len(), "nada depois do trailer");
                return found;
            }
            found.push((name, data, mode));
        }
    }

    fn header(ino: &str, mode: &str, size: &str, namesize: &str) -> String {
        // uid, gid, nlink = 1, mtime; depois dev e rdev (4 campos); namesize; check.
        let (zero, one) = ("00000000", "00000001");
        ["070701", ino, mode, zero, zero, one, zero, size]
            .into_iter()
            .chain([zero, zero, zero, zero, namesize, zero])
            .collect()
    }

    #[test]
    fn a_small_file_matches_the_golden_bytes() {
        // Cabeçalho 110 + "init\0" 5 = 115, pad 1 -> 116; dados 5 -> 121, pad 3 -> 124.
        // O trailer repete a conta: 124 + 110 + 11 = 245, pad 3 -> 248.
        let mut golden = Vec::new();
        golden.extend(header("00000001", "000081ED", "00000005", "00000005").bytes());
        golden.extend_from_slice(b"init\0\0hello\0\0\0");
        golden.extend(header("00000000", "00000000", "00000000", "0000000B").bytes());
        golden.extend_from_slice(b"TRAILER!!!\0\0\0\0");
        assert_eq!(golden.len(), 248);
        assert_eq!(build(&[("init", b"hello", 0o755)]).unwrap(), golden);
    }

    #[test]
    fn an_empty_list_is_just_the_trailer() {
        let archive = build(&[]).unwrap();
        assert_eq!(archive.len(), 124);
        assert_eq!(parse_back(&archive).len(), 0);
    }

    #[test]
    fn every_name_length_remainder_aligns_header_name_and_data() {
        let names = [
            "a", "ab", "abc", "abcd", "abcde", "abcdef", "abcdefg", "abcdefgh",
        ];
        for (len, name) in names.iter().enumerate() {
            for data_len in 0..9usize {
                let data = vec![0xA5; data_len];
                let archive = build(&[(name, &data, 0o644)]).unwrap();
                assert_eq!(
                    archive.len() % 4,
                    0,
                    "nome de {} bytes, dados {data_len}",
                    len + 1
                );
                assert_eq!(
                    parse_back(&archive),
                    [((*name).to_owned(), data, S_IFREG | 0o644)]
                );
            }
        }
    }

    #[test]
    fn nested_names_are_kept_whole_and_directories_are_not_written() {
        let archive = build(&[("bin/sh", b"x", 0o755)]).unwrap();
        assert_eq!(parse_back(&archive).len(), 1);
        assert_eq!(parse_back(&archive)[0].0, "bin/sh");
    }

    #[test]
    fn empty_and_large_files_round_trip_in_order() {
        let big: Vec<u8> = (0..1_000_003u32).map(|i| (i % 251) as u8).collect();
        let files: [File<'_>; 3] = [
            ("empty", b"", 0o600),
            ("big", &big, 0o755),
            ("last", b"z", 0o644),
        ];
        let parsed = parse_back(&build(&files).unwrap());
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0], ("empty".to_owned(), Vec::new(), S_IFREG | 0o600));
        assert_eq!(parsed[1].1, big);
        assert_eq!(parsed[2].0, "last");
    }

    #[test]
    fn inodes_are_sequential_and_the_rest_is_fixed() {
        let archive = build(&[("a", b"1", 0o644), ("b", b"2", 0o644)]).unwrap();
        // 110 + "a\0" = 112, mais "1" = 113, pad 3 -> 116.
        let second = 116;
        assert_eq!(&archive[6..14], b"00000001");
        assert_eq!(&archive[second + 6..second + 14], b"00000002");
        let fixed = |at: usize| &archive[at + 6 + 2 * 8..at + 6 + 7 * 8];
        let want = ["00000000", "00000000", "00000001", "00000000"].concat();
        assert_eq!(&fixed(0)[..32], want.as_bytes());
        assert_eq!(&fixed(second)[..32], want.as_bytes());
    }

    #[test]
    fn the_same_input_gives_the_same_bytes() {
        let files: [File<'_>; 2] = [("init", b"abc", 0o755), ("data", b"defgh", 0o644)];
        assert_eq!(build(&files).unwrap(), build(&files).unwrap());
    }

    #[test]
    fn each_invalid_name_is_refused() {
        let long = "x".repeat(4096);
        let cases = [
            "", "/init", "./init", "../init", "a/../b", "a/./b", "a//b", "dir/", ".", "..", "a\0b",
        ];
        assert_eq!(
            build(&[("TRAILER!!!", b"", 0o755)]),
            Err(Error::ReservedName("TRAILER!!!".to_owned()))
        );
        assert!(build(&[("a/TRAILER!!!", b"", 0o755)]).is_ok());
        for name in cases {
            assert_eq!(
                build(&[(name, b"x", 0o755)]),
                Err(Error::BadName(name.to_owned())),
                "{name:?}"
            );
        }
        assert_eq!(
            build(&[(&long, b"", 0o755)]),
            Err(Error::NameTooLong(long.clone()))
        );
        assert!(build(&[(&long[..4095], b"", 0o755)]).is_ok());
    }

    #[test]
    fn a_repeated_name_is_refused() {
        let files: [File<'_>; 3] = [
            ("init", b"a", 0o755),
            ("other", b"b", 0o755),
            ("init", b"c", 0o755),
        ];
        assert_eq!(build(&files), Err(Error::Duplicate("init".to_owned())));
    }

    #[test]
    fn a_mode_outside_the_permission_bits_is_refused() {
        assert_eq!(
            build(&[("init", b"", S_IFREG | 0o755)]),
            Err(Error::BadMode("init".to_owned(), S_IFREG | 0o755))
        );
    }

    #[test]
    fn the_command_line_takes_arch_out_and_named_inputs() {
        let args = |list: &[&str]| list.iter().map(|&arg| arg.to_owned()).collect::<Vec<_>>();
        let options = parse(&args(&[
            "--arch",
            "aarch64",
            "--out",
            "x.cpio",
            "init=/tmp/a",
        ]))
        .unwrap();
        assert_eq!(options.arches, [Arch::Aarch64]);
        assert_eq!(options.out, Some(PathBuf::from("x.cpio")));
        assert_eq!(
            split_input("init=/tmp/a").unwrap(),
            ("init".to_owned(), "/tmp/a")
        );
        assert_eq!(
            split_input("bin/sh=/tmp/a").unwrap(),
            ("bin/sh".to_owned(), "/tmp/a")
        );
        assert_eq!(
            split_input("/tmp/dir/prog").unwrap(),
            ("prog".to_owned(), "/tmp/dir/prog")
        );
        assert!(parse(&args(&["--arch", "all", "--out", "x", "f"])).is_err());
        assert!(parse(&args(&["--arch", "x86_64"])).is_err());
        assert!(parse(&args(&["--bogus", "f"])).is_err());
    }

    #[test]
    fn the_default_path_is_per_arch() {
        let path = default_path(Path::new("/r"), Arch::Aarch64);
        assert_eq!(path, Path::new("/r/target/initramfs/aarch64.cpio"));
    }

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rustos-initramfs-{tag}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn an_argument_that_exists_as_a_file_is_a_path_even_with_an_equals_sign() {
        let dir = scratch_dir("equals");
        let file = dir.join("a=b");
        fs::write(&file, b"x").unwrap();
        let input = file.to_str().unwrap();
        assert_eq!(split_input(input).unwrap(), ("a=b".to_owned(), input));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_named_input_with_a_missing_path_reports_that_path() {
        let dir = scratch_dir("missing");
        let missing = dir.join("nope");
        let out = dir.join("out.cpio");
        let input = format!("bin/sh={}", missing.display());
        let args = ["--out", out.to_str().unwrap(), &input].map(str::to_owned);
        let error = run(&dir, &args).unwrap_err();
        assert!(error.contains(&missing.display().to_string()), "{error}");
        assert!(!error.contains("bin/sh="), "{error}");
        assert!(!out.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_nested_name_reaches_the_archive_from_the_command_line() {
        let dir = scratch_dir("nested");
        let file = dir.join("busybox");
        let out = dir.join("out.cpio");
        fs::write(&file, b"elf").unwrap();
        let input = format!("bin/sh={}", file.display());
        let args = ["--out", out.to_str().unwrap(), &input].map(str::to_owned);
        run(&dir, &args).unwrap();
        let parsed = parse_back(&fs::read(&out).unwrap());
        assert_eq!(parsed[0].0, "bin/sh");
        assert_eq!(parsed[0].1, b"elf");
        fs::remove_dir_all(&dir).unwrap();
    }
}
