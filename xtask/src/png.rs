//! Codificador PNG mínimo, só `std`: deflate só com blocos "stored", sem compressão.

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const MAX_STORED: usize = 0xFFFF;

pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

pub fn adler32(data: &[u8]) -> u32 {
    const MODULUS: u32 = 65_521;
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % MODULUS;
        b = (b + a) % MODULUS;
    }
    b << 16 | a
}

/// Stream zlib com blocos stored de até 65535 bytes.
pub fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut blocks = data.chunks(MAX_STORED).peekable();
    if blocks.peek().is_none() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    }
    while let Some(block) = blocks.next() {
        out.push(u8::from(blocks.peek().is_none()));
        let length = u16::try_from(block.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&(!length).to_le_bytes());
        out.extend_from_slice(block);
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) -> Result<(), String> {
    let length = u32::try_from(data.len()).map_err(|_| "chunk PNG grande demais".to_owned())?;
    out.extend_from_slice(&length.to_be_bytes());
    let start = out.len();
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
    Ok(())
}

/// PNG de cor indexada (tipo 3, 8 bits); `pixels` são índices da paleta, linha a linha.
pub fn encode_indexed(
    width: usize,
    height: usize,
    palette: &[[u8; 3]],
    pixels: &[u8],
) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || pixels.len() != width * height {
        return Err(format!(
            "imagem {width}x{height} não bate com {} pixels",
            pixels.len()
        ));
    }
    if palette.is_empty() || palette.len() > 256 {
        return Err("paleta PNG precisa de 1 a 256 cores".to_owned());
    }
    if pixels
        .iter()
        .any(|&index| usize::from(index) >= palette.len())
    {
        return Err("pixel com índice fora da paleta".to_owned());
    }
    encode(
        width,
        height,
        COLOR_INDEXED,
        Some(&palette.concat()),
        pixels,
    )
}

/// PNG RGB de 8 bits por canal (tipo 2); `rgb` tem 3 bytes por pixel, linha a linha.
pub fn encode_rgb(width: usize, height: usize, rgb: &[u8]) -> Result<Vec<u8>, String> {
    if width == 0 || height == 0 || rgb.len() != width * height * 3 {
        return Err(format!(
            "imagem {width}x{height} não bate com {} bytes RGB",
            rgb.len()
        ));
    }
    encode(width, height, COLOR_RGB, None, rgb)
}

const COLOR_RGB: u8 = 2;
const COLOR_INDEXED: u8 = 3;

/// Cabeçalho, paleta opcional e linhas com filtro 0; `pixels` já validado por quem chama.
fn encode(
    width: usize,
    height: usize,
    color: u8,
    palette: Option<&[u8]>,
    pixels: &[u8],
) -> Result<Vec<u8>, String> {
    let wide = |value: usize| u32::try_from(value).map_err(|_| "imagem grande demais".to_owned());
    let mut header = Vec::new();
    header.extend_from_slice(&wide(width)?.to_be_bytes());
    header.extend_from_slice(&wide(height)?.to_be_bytes());
    header.extend_from_slice(&[8, color, 0, 0, 0]);
    let row_bytes = pixels.len() / height;
    let mut raw = Vec::with_capacity((row_bytes + 1) * height);
    for row in pixels.chunks(row_bytes) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut out = SIGNATURE.to_vec();
    chunk(&mut out, *b"IHDR", &header)?;
    if let Some(palette) = palette {
        chunk(&mut out, *b"PLTE", palette)?;
    }
    chunk(&mut out, *b"IDAT", &zlib_stored(&raw))?;
    chunk(&mut out, *b"IEND", &[])?;
    Ok(out)
}

/// Lê um PPM binário (P6, 8 bits), o formato do `screendump` da QEMU.
pub fn decode_ppm(bytes: &[u8]) -> Result<(usize, usize, &[u8]), String> {
    let mut fields = Vec::new();
    let mut at = 0;
    while fields.len() < 4 {
        while at < bytes.len() && bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if bytes.get(at) == Some(&b'#') {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        let start = at;
        while at < bytes.len() && !bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        if start == at {
            return Err("PPM truncado no cabeçalho".to_owned());
        }
        fields.push(String::from_utf8_lossy(&bytes[start..at]).into_owned());
    }
    let number = |text: &str| {
        text.parse::<usize>()
            .map_err(|_| format!("PPM: `{text}` não é número"))
    };
    if fields[0] != "P6" || fields[3] != "255" {
        return Err("só PPM P6 de 8 bits".to_owned());
    }
    let (width, height) = (number(&fields[1])?, number(&fields[2])?);
    let pixels = bytes.get(at + 1..).ok_or("PPM sem pixels")?;
    if pixels.len() < width * height * 3 {
        return Err("PPM com menos pixels que o cabeçalho".to_owned());
    }
    Ok((width, height, &pixels[..width * height * 3]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Desfaz `zlib_stored`; falha se algo fugir do que o codificador promete.
    pub fn inflate_stored(stream: &[u8]) -> Vec<u8> {
        assert_eq!(&stream[..2], &[0x78, 0x01]);
        let body = &stream[2..stream.len() - 4];
        let (mut out, mut at) = (Vec::new(), 0);
        loop {
            let last = body[at] == 1;
            assert!(body[at] <= 1, "só blocos stored");
            let length = usize::from(u16::from_le_bytes([body[at + 1], body[at + 2]]));
            let inverse = u16::from_le_bytes([body[at + 3], body[at + 4]]);
            assert_eq!(usize::from(!inverse), length);
            out.extend_from_slice(&body[at + 5..at + 5 + length]);
            at += 5 + length;
            if last {
                break;
            }
        }
        assert_eq!(at, body.len());
        let expected = u32::from_be_bytes(stream[stream.len() - 4..].try_into().unwrap());
        assert_eq!(adler32(&out), expected);
        out
    }

    fn chunks(png: &[u8]) -> Vec<(String, Vec<u8>)> {
        assert_eq!(&png[..8], &SIGNATURE);
        let (mut found, mut at) = (Vec::new(), 8);
        while at < png.len() {
            let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let kind = &png[at + 4..at + 8];
            let data = &png[at + 8..at + 8 + length];
            let crc =
                u32::from_be_bytes(png[at + 8 + length..at + 12 + length].try_into().unwrap());
            assert_eq!(crc32(&png[at + 4..at + 8 + length]), crc, "CRC de {kind:?}");
            found.push((String::from_utf8(kind.to_vec()).unwrap(), data.to_vec()));
            at += 12 + length;
        }
        found
    }

    #[test]
    fn crc32_matches_the_reference_vectors() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    #[test]
    fn adler32_matches_the_reference_vectors() {
        assert_eq!(adler32(b""), 1);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(b"a"), 0x0062_0062);
        assert_eq!(adler32(&vec![0; 70_000]), 4479 << 16 | 1);
    }

    #[test]
    fn zlib_stored_round_trips_small_empty_and_multi_block_data() {
        for size in [
            0,
            1,
            100,
            MAX_STORED - 1,
            MAX_STORED,
            MAX_STORED + 1,
            3 * MAX_STORED + 7,
        ] {
            let data: Vec<u8> = (0..size)
                .map(|index| u8::try_from(index % 251).unwrap())
                .collect();
            assert_eq!(inflate_stored(&zlib_stored(&data)), data, "tamanho {size}");
        }
    }

    #[test]
    fn zlib_stored_marks_only_the_last_block_as_final() {
        let stream = zlib_stored(&vec![7; MAX_STORED + 1]);
        assert_eq!(stream[2], 0);
        assert_eq!(stream[2 + 5 + MAX_STORED], 1);
        assert_eq!(zlib_stored(&[]).len(), 2 + 5 + 4);
    }

    #[test]
    fn encoded_png_has_the_expected_chunks_header_and_pixels() {
        let palette = [[1, 2, 3], [4, 5, 6]];
        let pixels = [0, 1, 1, 0, 1, 0];
        let png = encode_indexed(3, 2, &palette, &pixels).unwrap();
        let found = chunks(&png);
        let kinds: Vec<&str> = found.iter().map(|(kind, _)| kind.as_str()).collect();
        assert_eq!(kinds, ["IHDR", "PLTE", "IDAT", "IEND"]);
        assert_eq!(found[0].1, [0, 0, 0, 3, 0, 0, 0, 2, 8, 3, 0, 0, 0]);
        assert_eq!(found[1].1, [1, 2, 3, 4, 5, 6]);
        assert_eq!(inflate_stored(&found[2].1), [0, 0, 1, 1, 0, 0, 1, 0]);
        assert_eq!(found[3].1.len(), 0);
    }

    #[test]
    fn rgb_png_has_no_palette_and_keeps_three_bytes_per_pixel() {
        let png = encode_rgb(2, 1, &[1, 2, 3, 4, 5, 6]).unwrap();
        let found = chunks(&png);
        let kinds: Vec<&str> = found.iter().map(|(kind, _)| kind.as_str()).collect();
        assert_eq!(kinds, ["IHDR", "IDAT", "IEND"]);
        assert_eq!(found[0].1[9], COLOR_RGB);
        assert_eq!(inflate_stored(&found[1].1), [0, 1, 2, 3, 4, 5, 6]);
        assert!(encode_rgb(2, 1, &[0; 5]).is_err());
    }

    #[test]
    fn ppm_header_with_comment_is_parsed() {
        let ppm = b"P6\n# qemu\n2 1\n255\n\x01\x02\x03\x04\x05\x06";
        let (width, height, pixels) = decode_ppm(ppm).unwrap();
        assert_eq!((width, height), (2, 1));
        assert_eq!(pixels, [1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn ppm_rejects_other_formats_and_short_data() {
        assert!(decode_ppm(b"P5\n2 1\n255\n\x00\x00").is_err());
        assert!(decode_ppm(b"P6\n2 1\n255\n\x00").is_err());
        assert!(decode_ppm(b"P6\n2").is_err());
    }

    #[test]
    fn encoder_rejects_inconsistent_input() {
        let palette = [[0, 0, 0]];
        assert!(encode_indexed(0, 1, &palette, &[]).is_err());
        assert!(encode_indexed(1, 0, &palette, &[]).is_err());
        assert!(encode_indexed(2, 2, &palette, &[0; 3]).is_err());
        assert!(encode_indexed(1, 1, &palette, &[1]).is_err());
        assert!(encode_indexed(1, 1, &[], &[0]).is_err());
        assert!(encode_indexed(1, 1, &[[0, 0, 0]; 257], &[0]).is_err());
        assert!(encode_indexed(1, 1, &[[0, 0, 0]; 256], &[255]).is_ok());
    }
}
