//! Testes do leitor `newc`: bytes de ouro de um `cpio` real, arquivos sintéticos
//! e uma recusa por regra.

use super::*;

/// Gerado no desenvolvimento, nunca no build, com o `cpio` do host (bsdcpio 3.5.3,
/// macOS): `printf 'init\nbin\nbin/tool\n' | cpio -o -H newc`, em uma pasta com
/// `init` ("hi\n", modo 0755), o diretório `bin` e `bin/tool` ("ABCDEFGH").
const GOLDEN_HEX: &[&str] = &[
    "3037303730313032333961636238303030303831656430303030303166353030",
    "3030303030303030303030303031356530633062333030303030303030333030",
    "3030303030313030303030303064303030303030303030303030303030303030",
    "3030303030353030303030303030696e6974000068690a003037303730313032",
    "3339616362373030303034316564303030303031663530303030303030303030",
    "3030303030333565306330623330303030303030303030303030303030313030",
    "3030303030643030303030303030303030303030303030303030303030343030",
    "30303030303062696e0000003037303730313032333961636261303030303831",
    "6134303030303031663530303030303030303030303030303031356530633062",
    "3330303030303030303830303030303030313030303030303064303030303030",
    "303030303030303030303030303030303039303030303030303062696e2f746f",
    "6f6c000041424344454647483037303730313030303030303030303030303030",
    "3030303030303030303030303030303030303030303030303031303030303030",
    "3030303030303030303030303030303030303030303030303030303030303030",
    "3030303030303030303030303030303030623030303030303030545241494c45",
    "5221212100000000000000000000000000000000000000000000000000000000",
];

fn golden() -> Vec<u8> {
    let hex: String = GOLDEN_HEX.concat();
    (0..hex.len() / 2)
        .map(|index| u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap())
        .collect()
}

const REG: u32 = 0o100_644;
const DIR: u32 = 0o040_755;
const LNK: u32 = 0o120_777;

/// Montador de `newc` com o que o teste precisa quebrar.
struct Record<'a> {
    name: &'a [u8],
    mode: u32,
    nlink: u32,
    data: &'a [u8],
    extra_nuls: usize,
    upper: bool,
}

fn record<'a>(name: &'a [u8], mode: u32, data: &'a [u8]) -> Record<'a> {
    Record {
        name,
        mode,
        nlink: 1,
        data,
        extra_nuls: 0,
        upper: false,
    }
}

fn pad4(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn push(out: &mut Vec<u8>, record: &Record<'_>) {
    let namesize = record.name.len() + 1 + record.extra_nuls;
    let fields = [
        1,
        record.mode,
        0,
        0,
        record.nlink,
        0,
        u32::try_from(record.data.len()).unwrap(),
        0,
        0,
        0,
        0,
        u32::try_from(namesize).unwrap(),
        0,
    ];
    out.extend_from_slice(b"070701");
    for field in fields {
        let text = if record.upper {
            format!("{field:08X}")
        } else {
            format!("{field:08x}")
        };
        out.extend_from_slice(text.as_bytes());
    }
    out.extend_from_slice(record.name);
    out.extend(core::iter::repeat_n(0, 1 + record.extra_nuls));
    pad4(out);
    out.extend_from_slice(record.data);
    pad4(out);
}

fn archive(records: &[Record<'_>]) -> Vec<u8> {
    let mut out = Vec::new();
    for record in records {
        push(&mut out, record);
    }
    push(&mut out, &record(TRAILER, 0, b""));
    out
}

type Files = Vec<(Vec<u8>, Vec<u8>)>;

fn collect(bytes: &[u8]) -> Result<Files, Error> {
    Reader::new(bytes)
        .map(|entry| entry.map(|entry| (entry.name.to_vec(), entry.data.to_vec())))
        .collect()
}

fn first_error(bytes: &[u8]) -> Error {
    Reader::new(bytes)
        .find_map(Result::err)
        .expect("a recusa devia aparecer")
}

fn set_field(bytes: &mut [u8], index: usize, text: &[u8]) {
    bytes[6 + index * 8..6 + index * 8 + 8].copy_from_slice(text);
}

#[test]
fn golden_real_cpio_is_read() {
    let bytes = golden();
    assert_eq!(&bytes[..6], b"070701");
    let files = collect(&bytes).unwrap();
    assert_eq!(
        files,
        [
            (b"init".to_vec(), b"hi\n".to_vec()),
            (b"bin/tool".to_vec(), b"ABCDEFGH".to_vec()),
        ]
    );
    let init = find(&bytes, b"init").unwrap().unwrap();
    assert_eq!(init.mode, 0o100_755);
    assert_eq!(
        find(&bytes, b"bin/tool").unwrap().unwrap().data,
        b"ABCDEFGH"
    );
}

#[test]
fn find_misses_and_skips_directories() {
    let bytes = golden();
    assert_eq!(find(&bytes, b"absent").unwrap(), None);
    assert_eq!(find(&bytes, b"bin").unwrap(), None);
    assert_eq!(find(&bytes, b"").unwrap(), None);
}

#[test]
fn find_validates_the_whole_archive() {
    let mut bytes = archive(&[record(b"init", REG, b"x")]);
    bytes.truncate(bytes.len() - 4);
    assert!(find(&bytes, b"init").is_err());
}

#[test]
fn find_last_duplicate_wins() {
    let bytes = archive(&[record(b"a", REG, b"one"), record(b"a", REG, b"two")]);
    assert_eq!(find(&bytes, b"a").unwrap().unwrap().data, b"two");
}

#[test]
fn alignment_holds_for_every_name_and_data_length() {
    for name_len in 1..10 {
        for data_len in 0..10 {
            let name = vec![b'n'; name_len];
            let data = vec![b'd'; data_len];
            let bytes = archive(&[record(&name, REG, &data), record(b"after", REG, b"tail")]);
            assert_eq!(bytes.len() % 4, 0);
            let files = collect(&bytes).unwrap();
            assert_eq!(files[0], (name, data), "{name_len}/{data_len}");
            assert_eq!(files[1].1, b"tail");
        }
    }
}

#[test]
fn namesize_may_count_several_nuls_and_the_reader_advances_by_it() {
    for extra in 1..8 {
        let mut first = record(b"init", REG, b"abc");
        first.extra_nuls = extra;
        let bytes = archive(&[first, record(b"next", REG, b"z")]);
        let files = collect(&bytes).unwrap();
        assert_eq!(files.len(), 2, "extra {extra}");
        assert_eq!(files[0].0, b"init");
        assert_eq!(files[1], (b"next".to_vec(), b"z".to_vec()));
    }
}

#[test]
fn uppercase_hex_is_accepted() {
    let mut first = record(b"init", REG, b"abc");
    first.upper = true;
    let bytes = archive(&[first]);
    assert_eq!(find(&bytes, b"init").unwrap().unwrap().data, b"abc");
}

#[test]
fn empty_file_is_a_file() {
    let bytes = archive(&[record(b"empty", REG, b"")]);
    assert_eq!(collect(&bytes).unwrap(), [(b"empty".to_vec(), Vec::new())]);
}

#[test]
fn bytes_after_the_trailer_are_ignored() {
    let mut bytes = archive(&[record(b"init", REG, b"abc")]);
    bytes.extend_from_slice(&[0xff; 700]);
    assert_eq!(collect(&bytes).unwrap().len(), 1);
}

#[test]
fn directory_without_data_is_skipped() {
    let bytes = archive(&[
        record(b".", DIR, b""),
        record(b"bin", DIR, b""),
        record(b"bin/x", REG, b"1"),
    ]);
    assert_eq!(
        collect(&bytes).unwrap(),
        [(b"bin/x".to_vec(), b"1".to_vec())]
    );
}

#[test]
fn refuses_directory_with_data() {
    let bytes = archive(&[record(b"bin", DIR, b"oops")]);
    assert_eq!(first_error(&bytes), Error::DirectoryWithData);
}

#[test]
fn refuses_symlink() {
    let bytes = archive(&[record(b"link", LNK, b"init")]);
    assert_eq!(first_error(&bytes), Error::Symlink);
}

#[test]
fn refuses_devices_fifo_socket_and_unknown_types() {
    for mode in [0o020_600, 0o060_600, 0o010_600, 0o140_600, 0o030_000, 0] {
        let bytes = archive(&[record(b"dev", mode, b"")]);
        assert_eq!(first_error(&bytes), Error::SpecialFile, "{mode:o}");
    }
}

#[test]
fn refuses_hard_link_and_zero_link_count() {
    for nlink in [0, 2, 3, u32::MAX] {
        let mut file = record(b"init", REG, b"abc");
        file.nlink = nlink;
        assert_eq!(first_error(&archive(&[file])), Error::LinkCount, "{nlink}");
    }
}

#[test]
fn refuses_missing_trailer() {
    assert_eq!(first_error(&[]), Error::MissingTrailer);
    let mut bytes = Vec::new();
    push(&mut bytes, &record(b"init", REG, b"abc"));
    assert_eq!(first_error(&bytes), Error::MissingTrailer);
}

#[test]
fn refuses_trailer_with_data() {
    let mut bytes = Vec::new();
    push(&mut bytes, &record(TRAILER, 0, b"junk"));
    assert_eq!(first_error(&bytes), Error::BadTrailer);
}

#[test]
fn refuses_bad_magic() {
    for magic in [
        &b"070700"[..],
        b"07070 ",
        b"      ",
        b"\0\0\0\0\0\0",
        b"070701",
    ] {
        let mut bytes = archive(&[record(b"init", REG, b"abc")]);
        bytes[..6].copy_from_slice(magic);
        if magic == b"070701" {
            assert!(collect(&bytes).is_ok());
        } else {
            assert_eq!(first_error(&bytes), Error::BadMagic, "{magic:?}");
        }
    }
}

#[test]
fn refuses_checksum_format() {
    let mut bytes = archive(&[record(b"init", REG, b"abc")]);
    bytes[..6].copy_from_slice(b"070702");
    assert_eq!(first_error(&bytes), Error::ChecksumFormat);
}

#[test]
fn refuses_non_hex_digit_in_each_field() {
    for field in 0..13 {
        for bad in [b'g', b' ', b'-', b'+', b'x', 0u8] {
            let mut bytes = archive(&[record(b"init", REG, b"abc")]);
            bytes[6 + field * 8 + 3] = bad;
            assert_eq!(first_error(&bytes), Error::BadHex, "campo {field}");
        }
    }
}

#[test]
fn refuses_bad_name_size() {
    for text in [b"00000000", b"00001001", b"ffffffff", b"80000000"] {
        let mut bytes = archive(&[record(b"init", REG, b"abc")]);
        set_field(&mut bytes, 11, text);
        assert_eq!(first_error(&bytes), Error::BadNameSize, "{text:?}");
    }
}

#[test]
fn name_size_of_path_max_is_bounded_by_the_buffer() {
    let mut bytes = archive(&[record(b"init", REG, b"abc")]);
    set_field(&mut bytes, 11, b"00001000");
    assert_eq!(first_error(&bytes), Error::Truncated);
}

#[test]
fn refuses_name_size_past_the_end() {
    let mut bytes = archive(&[record(b"init", REG, b"abc")]);
    let len = u32::try_from(bytes.len()).unwrap();
    set_field(&mut bytes, 11, format!("{len:08x}").as_bytes());
    assert_eq!(first_error(&bytes), Error::Truncated);
}

#[test]
fn refuses_name_without_nul_and_empty_name() {
    let mut bytes = archive(&[record(b"init", REG, b"abc")]);
    bytes[HEADER_LEN + 4] = b'x';
    assert_eq!(first_error(&bytes), Error::BadName);
    let bytes = archive(&[record(b"", REG, b"abc")]);
    assert_eq!(first_error(&bytes), Error::BadName);
}

#[test]
fn refuses_file_size_past_the_end() {
    for text in [b"00000100", b"7fffffff", b"ffffffff", b"fffffffd"] {
        let mut bytes = archive(&[record(b"init", REG, b"abc")]);
        set_field(&mut bytes, 6, text);
        assert_eq!(first_error(&bytes), Error::Truncated, "{text:?}");
    }
}

#[test]
fn overflow_in_size_fields_never_panics() {
    for field in [6, 11] {
        for text in [
            b"ffffffff",
            b"fffffffc",
            b"fffffffd",
            b"7fffffff",
            b"80000000",
        ] {
            let mut bytes = archive(&[record(b"init", REG, b"abc")]);
            set_field(&mut bytes, field, text);
            assert!(Reader::new(&bytes).any(|entry| entry.is_err()));
        }
    }
}

#[test]
fn every_truncation_of_a_valid_archive_is_an_error_or_valid() {
    let bytes = golden();
    let trailer_end = bytes
        .windows(TRAILER.len())
        .position(|window| window == TRAILER)
        .unwrap()
        + TRAILER.len()
        + 1;
    for len in 0..bytes.len() {
        let prefix = &bytes[..len];
        let result = collect(prefix);
        if len < trailer_end {
            assert!(result.is_err(), "len {len}");
        } else {
            assert!(result.is_ok(), "len {len}");
        }
    }
}

#[test]
fn every_truncation_of_a_synthetic_archive_is_an_error() {
    let bytes = archive(&[
        record(b"dir", DIR, b""),
        record(b"a-long-name", REG, b"payload"),
    ]);
    for len in 0..bytes.len() - 7 {
        assert!(collect(&bytes[..len]).is_err(), "len {len}");
    }
}

#[test]
fn single_byte_corruption_never_panics() {
    let base = golden();
    for index in 0..base.len() {
        for value in [0u8, b'0', b'f', b'G', 0xff, 0x7f] {
            let mut bytes = base.clone();
            bytes[index] = value;
            let _ = collect(&bytes);
            let _ = find(&bytes, b"init");
        }
    }
}

#[test]
fn iterator_stops_after_the_first_error() {
    let bytes = archive(&[record(b"link", LNK, b"x"), record(b"init", REG, b"abc")]);
    let mut reader = Reader::new(&bytes);
    assert_eq!(reader.next(), Some(Err(Error::Symlink)));
    assert_eq!(reader.next(), None);
    assert_eq!(reader.next(), None);
}

#[test]
fn iterator_stops_after_the_trailer() {
    let bytes = archive(&[record(b"init", REG, b"abc")]);
    let mut reader = Reader::new(&bytes);
    assert!(matches!(reader.next(), Some(Ok(_))));
    assert_eq!(reader.next(), None);
    assert_eq!(reader.next(), None);
}

#[test]
fn header_len_is_6_plus_13_fields_of_8() {
    assert_eq!(HEADER_LEN, 110);
}
