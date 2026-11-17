//! Vetores do BLAKE2b: RFC 7693 (apêndice A) e os oficiais do repositório de
//! referência, em `tests/data/blake2b-kat.txt` (origem no cabeçalho do arquivo).

use rustos_crypto::blake2b::{self, Blake2b, Error};

const KAT: &str = include_str!("data/blake2b-kat.txt");

fn hex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap())
        .collect()
}

fn sequential(len: usize) -> Vec<u8> {
    (0..len).map(|i| i.to_le_bytes()[0]).collect()
}

fn digest(out_len: usize, key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut out = vec![0_u8; out_len];
    blake2b::hash(&mut out, key, data).unwrap();
    out
}

fn kat(section: &str) -> Vec<Vec<u8>> {
    KAT.lines()
        .filter_map(|line| line.strip_prefix(section)?.strip_prefix(' '))
        .map(hex)
        .collect()
}

#[test]
fn abc_matches_rfc_7693_appendix_a() {
    let expected = "ba80a53f981c4d0d6a2797b69f12f6e94c212f14685ac4b74b12bb6fdbffa2d1\
                    7d87c5392aab792dc252d5de4533cc9518d38aa8dbf1925ab92386edd4009923";
    assert_eq!(digest(64, &[], b"abc"), hex(expected));
}

#[test]
fn unkeyed_matches_the_official_kat_for_inputs_of_0_to_255_bytes() {
    let expected = kat("unkeyed");
    assert_eq!(expected.len(), 256);
    for (len, want) in expected.iter().enumerate() {
        assert_eq!(&digest(64, &[], &sequential(len)), want, "len {len}");
    }
}

#[test]
fn keyed_matches_the_official_kat_for_inputs_of_0_to_255_bytes() {
    let expected = kat("keyed");
    assert_eq!(expected.len(), 256);
    let key = sequential(64);
    for (len, want) in expected.iter().enumerate() {
        assert_eq!(&digest(64, &key, &sequential(len)), want, "len {len}");
    }
}

// Os valores abaixo vêm de `hashlib.blake2b` do CPython (implementação de
// referência do BLAKE2); o KAT oficial só cobre saída de 64 bytes.
#[test]
fn shorter_outputs_match_the_reference_implementation() {
    let cases = [
        (1, "6b"),
        (20, "384264f676f39536840523f284921cdc68b6846b"),
        (
            32,
            "bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319",
        ),
        (
            48,
            "6f56a82c8e7ef526dfe182eb5212f7db9df1317e57815dbda46083fc30f54ee6\
             c66ba83be64b302d7cba6ce15bb556f4",
        ),
    ];
    for (len, want) in cases {
        assert_eq!(digest(len, &[], b"abc"), hex(want), "out_len {len}");
    }
    assert_eq!(digest(1, &[], &sequential(128)), hex("f4"));
}

#[test]
fn keys_of_other_lengths_match_the_reference_implementation() {
    let short = "d63a32d3e44738d7907f964316c241adaba0abfeabc32349677578a15a203f7f";
    assert_eq!(digest(32, &sequential(32), b"abc"), hex(short));
    let mixed = "8c95c52cb158a8bff72d3c6f321649fb36764d75ac300b3bad719b567c6169e3\
                 249604e0f7e4e8d96740f3edd7799c16";
    assert_eq!(digest(48, &sequential(17), &sequential(200)), hex(mixed));
}

#[test]
fn update_in_any_split_equals_the_one_shot_hash() {
    let key = sequential(20);
    for len in [0, 1, 127, 128, 129, 255, 256, 257, 300] {
        let data = sequential(len);
        let whole = digest(40, &key, &data);
        for cut in 0..=len {
            let mut state = Blake2b::new(40, &key).unwrap();
            state.update(&data[..cut]);
            state.update(&data[cut..]);
            let mut out = [0_u8; 40];
            state.finalize(&mut out).unwrap();
            assert_eq!(out.to_vec(), whole, "len {len} cut {cut}");
        }
    }
}

#[test]
fn update_byte_by_byte_equals_the_one_shot_hash() {
    let data = sequential(300);
    let mut state = Blake2b::new(64, &[]).unwrap();
    for byte in &data {
        state.update(core::slice::from_ref(byte));
    }
    let mut out = [0_u8; 64];
    state.finalize(&mut out).unwrap();
    assert_eq!(out.to_vec(), digest(64, &[], &data));
}

#[test]
fn output_length_outside_1_to_64_is_refused() {
    assert_eq!(Blake2b::new(0, &[]).unwrap_err(), Error::OutputLength);
    assert_eq!(Blake2b::new(65, &[]).unwrap_err(), Error::OutputLength);
    assert_eq!(
        blake2b::hash(&mut [], &[], b"x").unwrap_err(),
        Error::OutputLength
    );
    assert!(Blake2b::new(1, &[]).is_ok());
    assert!(Blake2b::new(64, &[]).is_ok());
}

#[test]
fn key_longer_than_64_bytes_is_refused() {
    assert_eq!(Blake2b::new(32, &[0; 65]).unwrap_err(), Error::KeyLength);
    assert!(Blake2b::new(32, &[0; 64]).is_ok());
}

#[test]
fn finalize_refuses_a_buffer_of_the_wrong_size() {
    let state = Blake2b::new(32, &[]).unwrap();
    assert_eq!(
        state.finalize(&mut [0; 31]).unwrap_err(),
        Error::OutputLength
    );
    let state = Blake2b::new(32, &[]).unwrap();
    assert_eq!(
        state.finalize(&mut [0; 33]).unwrap_err(),
        Error::OutputLength
    );
}

#[test]
fn debug_hides_the_state() {
    let state = Blake2b::new(32, &[0xAB; 16]).unwrap();
    let text = format!("{state:?}");
    assert!(text.contains("<redacted>"));
    assert!(!text.contains("171"));
}
