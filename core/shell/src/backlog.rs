//! Saída que não é do shell e espera o momento de entrar na tela (SDD 16, "Saída de segundo plano").

use core::fmt::{self, Write};
use core::str;

/// Fila limitada de texto de fora do shell. Cada `push` entra inteiro ou não entra: uma linha
/// nunca sai pela metade, e o que não coube é contado, não adivinhado.
#[derive(Clone, Debug)]
pub struct Backlog<const N: usize> {
    bytes: [u8; N],
    len: usize,
    lost: u32,
}

impl<const N: usize> Default for Backlog<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Escreve no fim da fila e falha, sem escrever nada além, quando não cabe.
struct Tail<'a> {
    bytes: &'a mut [u8],
    len: &'a mut usize,
    overflowed: bool,
}

impl Write for Tail<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len.saturating_add(text.len());
        let Some(room) = self.bytes.get_mut(*self.len..end) else {
            self.overflowed = true;
            return Err(fmt::Error);
        };
        room.copy_from_slice(text.as_bytes());
        *self.len = end;
        Ok(())
    }
}

impl<const N: usize> Backlog<N> {
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            len: 0,
            lost: 0,
        }
    }

    /// Há texto ou perda a mostrar.
    pub const fn is_empty(&self) -> bool {
        self.len == 0 && self.lost == 0
    }

    /// Guarda `args` inteiro. Cheia, descarta o `push` inteiro e conta a perda; devolve se guardou.
    pub fn push(&mut self, args: fmt::Arguments<'_>) -> bool {
        self.push_with(|out| out.write_fmt(args))
    }

    /// Como `push`, para quem produz o texto por um adaptador (o filtro de endereços do kernel),
    /// que pode engolir o erro de falta de espaço: a perda se vê pelo estouro, não pelo retorno.
    pub fn push_with(&mut self, write: impl FnOnce(&mut dyn Write) -> fmt::Result) -> bool {
        let before = self.len;
        let mut tail = Tail {
            bytes: &mut self.bytes,
            len: &mut self.len,
            overflowed: false,
        };
        let done = write(&mut tail).is_ok() && !tail.overflowed;
        if done {
            return true;
        }
        self.len = before;
        self.lost = self.lost.saturating_add(1);
        false
    }

    /// Escreve em `out` o que guardou e, se algo se perdeu, uma linha que o diz; esvazia a fila.
    pub fn drain(&mut self, out: &mut dyn Write) -> fmt::Result {
        let text = str::from_utf8(&self.bytes[..self.len]).unwrap_or("");
        let lost = self.lost;
        (self.len, self.lost) = (0, 0);
        out.write_str(text)?;
        if lost > 0 {
            writeln!(
                out,
                "[rustos] user output: {lost} writes lost (backlog full)"
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drained<const N: usize>(backlog: &mut Backlog<N>) -> String {
        let mut text = String::new();
        backlog.drain(&mut text).unwrap();
        text
    }

    #[test]
    fn a_push_comes_back_whole_and_in_order() {
        let mut backlog = Backlog::<64>::new();
        assert!(backlog.is_empty());
        assert!(backlog.push(format_args!("[user] a\n")));
        assert!(backlog.push(format_args!("[user] {}\n", "b")));
        assert!(!backlog.is_empty());
        assert_eq!(drained(&mut backlog), "[user] a\n[user] b\n");
        assert!(backlog.is_empty());
    }

    #[test]
    fn a_push_that_does_not_fit_is_dropped_whole_and_counted() {
        let mut backlog = Backlog::<16>::new();
        assert!(backlog.push(format_args!("0123456789\n")));
        assert!(!backlog.push(format_args!("abcdefgh\n")));
        assert!(!backlog.push(format_args!("x{}\n", "y".repeat(40))));
        assert_eq!(
            drained(&mut backlog),
            "0123456789\n[rustos] user output: 2 writes lost (backlog full)\n"
        );
    }

    #[test]
    fn a_small_push_still_fits_after_a_big_one_was_dropped() {
        let mut backlog = Backlog::<16>::new();
        assert!(backlog.push(format_args!("0123456789\n")));
        assert!(!backlog.push(format_args!("abcdefgh\n")));
        assert!(backlog.push(format_args!("ok\n")));
        assert!(drained(&mut backlog).starts_with("0123456789\nok\n[rustos]"));
    }

    #[test]
    fn a_writer_that_swallows_the_overflow_is_still_rolled_back() {
        let mut backlog = Backlog::<8>::new();
        assert!(backlog.push(format_args!("ab\n")));
        let kept = backlog.push_with(|out| {
            out.write_str("123").ok();
            out.write_str("4567890").ok();
            Ok(())
        });
        assert!(!kept);
        assert!(drained(&mut backlog).starts_with("ab\n[rustos]"));
    }

    #[test]
    fn a_push_that_fills_it_exactly_fits() {
        let mut backlog = Backlog::<4>::new();
        assert!(backlog.push(format_args!("abc\n")));
        assert!(!backlog.push(format_args!("\n")));
        assert!(backlog.push(format_args!("")));
    }

    #[test]
    fn a_loss_alone_is_not_empty_and_drains_the_notice() {
        let mut backlog = Backlog::<2>::new();
        assert!(!backlog.push(format_args!("long\n")));
        assert!(!backlog.is_empty());
        assert_eq!(
            drained(&mut backlog),
            "[rustos] user output: 1 writes lost (backlog full)\n"
        );
        assert!(backlog.is_empty());
        assert_eq!(drained(&mut backlog), "");
    }
}
