//! Como um endereço do kernel aparece num log: como deslocamento dentro de uma região
//! (`kernel+0x1234`), nunca absoluto, para não entregar o KASLR (SDD 09).

use core::fmt::{self, Write};

/// Primeiro endereço da metade alta canônica (4 níveis).
pub const HIGH_HALF: u64 = 0xffff_8000_0000_0000;
/// Do último 64 KiB em diante não há endereço de kernel: é `-1`, `u64::MAX` e parentes.
const CEILING: u64 = 0xffff_ffff_ffff_0000;
/// Onde o linker script põe a imagem; o Limine soma o sorteio a esta base.
pub const LINK_BASE: u64 = 0xffff_ffff_8000_0000;
/// Janela do topo em que o Limine sorteia a base da imagem (`elf.c`, `elf64_load`).
pub const SLIDE_WINDOW: u64 = 1 << 31;
/// Passo do sorteio: o do alinhamento de página, o menor que o Limine aceita.
pub const SLIDE_STEP: u64 = 4096;

/// Quantos bits de sorteio cabem na janela depois de a imagem de `image_len` bytes entrar.
pub const fn slide_bits(image_len: u64) -> u32 {
    let room = SLIDE_WINDOW.saturating_sub(image_len);
    match (room / SLIDE_STEP).checked_ilog2() {
        Some(bits) => bits,
        None => 0,
    }
}

/// Uma faixa virtual com nome: `[start, start + len)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub name: &'static str,
    pub start: u64,
    pub len: u64,
}

impl Region {
    pub const EMPTY: Self = Self {
        name: "",
        start: 0,
        len: 0,
    };

    const fn offset_of(&self, addr: u64) -> Option<u64> {
        match addr.checked_sub(self.start) {
            Some(offset) if offset < self.len => Some(offset),
            _ => None,
        }
    }
}

/// As regiões que o log sabe nomear. Preencher as do kernel de cada máquina é do `kernel/`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Regions {
    list: [Region; 4],
}

/// Como um endereço aparece.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shown {
    /// Dentro de uma região conhecida: nome e deslocamento.
    Offset(&'static str, u64),
    /// Metade alta fora de toda região: não se diz nada.
    Hidden,
    /// Não é endereço de kernel (metade baixa, `-1`): sai como veio.
    Plain(u64),
}

impl Regions {
    pub const fn new(list: [Region; 4]) -> Self {
        Self { list }
    }

    pub const fn empty() -> Self {
        Self::new([Region::EMPTY; 4])
    }

    pub fn show(&self, addr: u64) -> Shown {
        if !(HIGH_HALF..CEILING).contains(&addr) {
            return Shown::Plain(addr);
        }
        for region in &self.list {
            if let Some(offset) = region.offset_of(addr) {
                return Shown::Offset(region.name, offset);
            }
        }
        Shown::Hidden
    }

    /// O que `{}` imprime para `addr`.
    pub const fn display(&self, addr: u64) -> Redacted<'_> {
        Redacted {
            addr,
            regions: self,
        }
    }
}

/// `Display` de um endereço pelas regras de `Regions::show`.
#[derive(Clone, Copy, Debug)]
pub struct Redacted<'a> {
    addr: u64,
    regions: &'a Regions,
}

impl fmt::Display for Redacted<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.regions.show(self.addr) {
            Shown::Offset(name, offset) => write!(f, "{name}+{offset:#x}"),
            Shown::Hidden => f.write_str("<kernel>"),
            Shown::Plain(addr) => write!(f, "{addr:#x}"),
        }
    }
}

/// `0x` mais 16 dígitos: o maior literal que cabe num `u64`.
const MAX_LITERAL: usize = 18;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Text,
    Zero,
    Hex,
}

/// Escreve em `out` o que recebe, trocando todo literal `0x…` de endereço do kernel pela
/// forma de `Regions::show`. Pega o que `{:p}`, `{:#x}` e o `Debug` de endereço imprimem,
/// mesmo quando o texto chega em pedaços; é a rede de segurança, não a regra.
pub struct Scrubber<'a, W: Write> {
    out: W,
    regions: &'a Regions,
    state: State,
    pending: [u8; MAX_LITERAL],
    len: usize,
}

impl<'a, W: Write> Scrubber<'a, W> {
    pub const fn new(out: W, regions: &'a Regions) -> Self {
        Self {
            out,
            regions,
            state: State::Text,
            pending: [0; MAX_LITERAL],
            len: 0,
        }
    }

    fn push(&mut self, byte: u8) {
        self.pending[self.len] = byte;
        self.len += 1;
    }

    fn emit_pending(&mut self) -> fmt::Result {
        let text = core::str::from_utf8(&self.pending[..self.len]).unwrap_or("");
        self.len = 0;
        self.state = State::Text;
        self.out.write_str(text)
    }

    /// Fecha o literal acumulado: endereço de kernel vira `Redacted`; o resto sai igual.
    fn close_literal(&mut self) -> fmt::Result {
        let digits = &self.pending[2..self.len];
        let value = digits.iter().try_fold(0u64, |acc, &digit| {
            let nibble = char::from(digit).to_digit(16)?;
            acc.checked_mul(16)?.checked_add(u64::from(nibble))
        });
        match value {
            Some(addr) if digits.len() >= 9 && (HIGH_HALF..CEILING).contains(&addr) => {
                self.len = 0;
                self.state = State::Text;
                write!(self.out, "{}", self.regions.display(addr))
            }
            _ => self.emit_pending(),
        }
    }

    /// Solta o que ficou à espera de um byte que decida; chamar no fim do texto.
    pub fn finish(&mut self) -> fmt::Result {
        match self.state {
            State::Text => Ok(()),
            State::Zero => self.emit_pending(),
            State::Hex => self.close_literal(),
        }
    }
}

impl<W: Write> Write for Scrubber<'_, W> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let bytes = text.as_bytes();
        let mut clean_from = 0;
        let mut at = 0;
        while at < bytes.len() {
            let byte = bytes[at];
            match self.state {
                State::Text if byte == b'0' => {
                    self.out.write_str(&text[clean_from..at])?;
                    self.push(byte);
                    self.state = State::Zero;
                    clean_from = at + 1;
                }
                State::Text => {}
                State::Zero if byte == b'x' => {
                    self.push(byte);
                    self.state = State::Hex;
                    clean_from = at + 1;
                }
                State::Hex if byte.is_ascii_hexdigit() && self.len < MAX_LITERAL => {
                    self.push(byte);
                    clean_from = at + 1;
                }
                State::Zero => {
                    self.emit_pending()?;
                    continue;
                }
                State::Hex if byte.is_ascii_hexdigit() => {
                    self.emit_pending()?;
                    continue;
                }
                State::Hex => {
                    self.close_literal()?;
                    continue;
                }
            }
            at += 1;
        }
        self.out.write_str(&text[clean_from..])
    }
}

impl<W: Write> fmt::Debug for Scrubber<'_, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Scrubber").finish_non_exhaustive()
    }
}

impl<W: Write> Drop for Scrubber<'_, W> {
    fn drop(&mut self) {
        self.finish().ok();
    }
}

#[cfg(test)]
mod tests {
    use std::string::String;

    use super::*;

    fn regions() -> Regions {
        Regions::new([
            Region {
                name: "kernel",
                start: 0xffff_ffff_9b7a_5000,
                len: 0x4_0000,
            },
            Region {
                name: "direct",
                start: 0xffff_8468_4000_0000,
                len: 1 << 39,
            },
            Region::EMPTY,
            Region::EMPTY,
        ])
    }

    fn scrub(chunks: &[&str]) -> String {
        let regions = regions();
        let mut out = String::new();
        {
            let mut scrubber = Scrubber::new(&mut out, &regions);
            for chunk in chunks {
                scrubber.write_str(chunk).unwrap();
            }
        }
        out
    }

    #[test]
    fn an_address_inside_a_region_shows_the_offset_only() {
        let regions = regions();
        let shown = std::format!("{}", regions.display(0xffff_ffff_9b7a_6234));
        assert_eq!(shown, "kernel+0x1234");
        let heap = std::format!("{}", regions.display(0xffff_8468_4000_1000));
        assert_eq!(heap, "direct+0x1000");
    }

    #[test]
    fn a_high_half_address_outside_every_region_is_hidden() {
        let regions = regions();
        assert_eq!(regions.show(0xffff_fe80_0000_1000), Shown::Hidden);
        assert_eq!(
            std::format!("{}", regions.display(0xffff_fe80_0000_1000)),
            "<kernel>"
        );
    }

    #[test]
    fn values_that_are_not_kernel_addresses_come_out_as_they_are() {
        let regions = regions();
        assert_eq!(regions.show(0), Shown::Plain(0));
        assert_eq!(
            regions.show(0x7fff_ffff_f000),
            Shown::Plain(0x7fff_ffff_f000)
        );
        assert_eq!(regions.show(u64::MAX), Shown::Plain(u64::MAX));
        assert_eq!(
            regions.show(u64::MAX - 0xfff),
            Shown::Plain(u64::MAX - 0xfff)
        );
    }

    #[test]
    fn the_region_edges_are_half_open() {
        let regions = regions();
        assert_eq!(
            regions.show(0xffff_ffff_9b7a_5000),
            Shown::Offset("kernel", 0)
        );
        assert_eq!(
            regions.show(0xffff_ffff_9b7e_4fff),
            Shown::Offset("kernel", 0x3_ffff)
        );
        assert_eq!(regions.show(0xffff_ffff_9b7e_5000), Shown::Hidden);
    }

    #[test]
    fn the_scrubber_rewrites_a_literal_in_the_middle_of_a_line() {
        let out = scrub(&["heap: dealloc de 0xffffffff9b7a7000 falhou\n"]);
        assert_eq!(out, "heap: dealloc de kernel+0x2000 falhou\n");
    }

    #[test]
    fn the_scrubber_follows_a_literal_split_across_writes() {
        let out = scrub(&["at 0", "x", "ffff8468", "40001000", ")\n"]);
        assert_eq!(out, "at direct+0x1000)\n");
    }

    #[test]
    fn the_scrubber_flushes_a_literal_that_ends_the_text() {
        assert_eq!(scrub(&["pc 0xffffffff9b7a5010"]), "pc kernel+0x10");
        assert_eq!(scrub(&["mask 0x"]), "mask 0x");
    }

    #[test]
    fn the_scrubber_leaves_alone_what_is_not_an_address() {
        for text in [
            "mask 0xf\n",
            "random 0x00000000000000ff\n",
            "0xffffffffffffffff\n",
            "0x7fffffff1000 user\n",
            "0xffff8468400010001 too long\n",
            "100 0x 05 0y 00x1\n",
            "ação 0xé\n",
        ] {
            assert_eq!(scrub(&[text]), text, "{text:?}");
        }
    }

    #[test]
    fn the_scrubber_handles_two_literals_back_to_back() {
        let out = scrub(&["0xffffffff9b7a5000,0xfffffe8000000000\n"]);
        assert_eq!(out, "kernel+0x0,<kernel>\n");
    }

    #[test]
    fn a_zero_before_the_literal_does_not_hide_it() {
        assert_eq!(scrub(&["00xffffffff9b7a5000"]), "0kernel+0x0");
    }

    #[test]
    fn the_slide_entropy_follows_the_room_the_image_leaves() {
        assert_eq!(slide_bits(180 * 1024), 18);
        assert_eq!(slide_bits(0), 19);
        assert_eq!(slide_bits(SLIDE_WINDOW), 0);
    }
}
