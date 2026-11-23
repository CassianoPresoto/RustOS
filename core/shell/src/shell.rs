//! Núcleo do shell: liga decodificador, linha, histórico e comandos, e redesenha a linha.

use core::cmp::Ordering;
use core::fmt;

use crate::ansi::{ACCENT, BEL, CLEAR_SCREEN, RESET};
use crate::backlog::Backlog;
use crate::commands::{self, COMMANDS, Ctx};
use crate::decoder::{Decoder, Key};
use crate::history::History;
use crate::host::Host;
use crate::line::LineBuffer;
use crate::parse;

const PROMPT: &str = "\x1b[1;38;5;208mrustos\x1b[0m \x1b[34m~\x1b[0m % ";
const PROMPT_WIDTH: usize = 11;
/// Escreve um espaço na última coluna para forçar a quebra, volta e apaga o espaço: o cursor
/// fica na coluna 0 da próxima linha, com quebra adiada ou imediata.
const WRAP_FIX: &str = " \r\x1b[K";
const DEFAULT_COLS: usize = 80;

/// Shell embutido: edição de linha estilo readline, histórico e comandos.
/// `LINE` é o tamanho da linha em bytes; `HISTORY`, o número de linhas lembradas.
/// A saída usa CR LF e supõe uma célula por caractere.
#[derive(Clone, Debug)]
pub struct Shell<const LINE: usize, const HISTORY: usize> {
    decoder: Decoder,
    line: LineBuffer<LINE>,
    draft: LineBuffer<LINE>,
    history: History<LINE, HISTORY>,
    recall: Option<usize>,
    /// Células entre o início do prompt e o cursor na tela, contadas em linhas de `cols`.
    at: usize,
}

impl<const LINE: usize, const HISTORY: usize> Default for Shell<LINE, HISTORY> {
    fn default() -> Self {
        Self::new()
    }
}

fn terminal_cols(host: &dyn Host) -> usize {
    match host.terminal_size().0 {
        0 => DEFAULT_COLS,
        cols => usize::from(cols),
    }
}

fn move_cursor(out: &mut dyn fmt::Write, count: usize, direction: char) -> fmt::Result {
    match count {
        0 => Ok(()),
        1 => write!(out, "\x1b[{direction}"),
        _ => write!(out, "\x1b[{count}{direction}"),
    }
}

/// Entre os `names` que começam por `typed`: quantos são, o primeiro e o prefixo comum.
fn analyze<'a>(typed: &str, names: impl Iterator<Item = &'a str>) -> (usize, &'a str, usize) {
    let (mut count, mut first, mut common) = (0, "", 0);
    for name in names.filter(|name| name.starts_with(typed)) {
        if count == 0 {
            (first, common) = (name, name.len());
        } else {
            let same = first
                .bytes()
                .zip(name.bytes())
                .take_while(|(a, b)| a == b)
                .count();
            common = common.min(same);
        }
        count += 1;
    }
    (count, first, common)
}

impl<const LINE: usize, const HISTORY: usize> Shell<LINE, HISTORY> {
    /// Shell vazio; cabe num `static`.
    pub const fn new() -> Self {
        Self {
            decoder: Decoder::new(),
            line: LineBuffer::new(),
            draft: LineBuffer::new(),
            history: History::new(),
            recall: None,
            at: 0,
        }
    }

    /// Escreve o banner curto e o primeiro prompt.
    pub fn start(&mut self, host: &impl Host, out: &mut impl fmt::Write) -> fmt::Result {
        let info = host.info();
        write!(
            out,
            "{ACCENT}{}{RESET} {} ({})\r\nType 'help' to list the commands.\r\n",
            info.os_name, info.os_version, info.arch
        )?;
        self.decoder = Decoder::new();
        self.reset_line();
        self.redraw(terminal_cols(host), out)
    }

    /// Consome um byte de entrada VT e escreve o que mudou na tela.
    pub fn feed(
        &mut self,
        byte: u8,
        host: &mut impl Host,
        out: &mut impl fmt::Write,
    ) -> fmt::Result {
        match self.decoder.push(byte) {
            Some(key) => self.dispatch(key, &*host, out),
            None => Ok(()),
        }
    }

    /// Mostra o que `backlog` guardou sem rasgar a linha em edição: apaga o prompt, deixa o
    /// texto entrar a partir da coluna 0 e redesenha prompt e linha onde o cursor estava.
    pub fn interject<const N: usize>(
        &mut self,
        host: &impl Host,
        backlog: &mut Backlog<N>,
        out: &mut impl fmt::Write,
    ) -> fmt::Result {
        if backlog.is_empty() {
            return Ok(());
        }
        let cols = terminal_cols(host);
        move_cursor(out, self.at / cols, 'A')?;
        out.write_str("\r\x1b[J")?;
        self.at = 0;
        backlog.drain(out)?;
        self.redraw(cols, out)
    }

    fn reset_line(&mut self) {
        self.line.clear();
        self.draft.clear();
        self.recall = None;
        self.at = 0;
    }

    fn dispatch(&mut self, key: Key, host: &dyn Host, out: &mut dyn fmt::Write) -> fmt::Result {
        let cols = terminal_cols(host);
        match key {
            Key::Char(c) => self.insert(c, cols, out),
            Key::Enter => self.submit(host, cols, out),
            Key::Tab => self.complete(cols, out),
            Key::Backspace => self.backspace(cols, out),
            Key::EndOfFile if self.line.is_empty() => Ok(()),
            Key::Delete | Key::EndOfFile => self.edit(LineBuffer::delete, cols, out),
            Key::Left => self.navigate(LineBuffer::left, cols, out),
            Key::Right => self.navigate(LineBuffer::right, cols, out),
            Key::Home => self.navigate(LineBuffer::home, cols, out),
            Key::End => self.navigate(LineBuffer::end, cols, out),
            Key::KillToEnd if self.line.kill_to_end() => out.write_str("\x1b[J"),
            Key::KillToEnd => Ok(()),
            Key::KillToStart => self.edit(LineBuffer::kill_to_start, cols, out),
            Key::KillWord => self.edit(LineBuffer::kill_word, cols, out),
            Key::ClearScreen => {
                out.write_str(CLEAR_SCREEN)?;
                self.at = 0;
                self.redraw(cols, out)
            }
            Key::Interrupt => {
                self.move_to_end(cols, out)?;
                out.write_str("^C\r\n")?;
                self.reset_line();
                self.redraw(cols, out)
            }
            Key::Up => self.recall_older(cols, out),
            Key::Down => self.recall_newer(cols, out),
        }
    }

    /// Aplica uma edição e redesenha a linha se ela mudou.
    fn edit(
        &mut self,
        change: fn(&mut LineBuffer<LINE>) -> bool,
        cols: usize,
        out: &mut dyn fmt::Write,
    ) -> fmt::Result {
        if change(&mut self.line) {
            self.redraw(cols, out)
        } else {
            Ok(())
        }
    }

    /// Anda o cursor da linha e leva o da tela junto.
    fn navigate(
        &mut self,
        step: fn(&mut LineBuffer<LINE>) -> bool,
        cols: usize,
        out: &mut dyn fmt::Write,
    ) -> fmt::Result {
        if step(&mut self.line) {
            self.move_to(PROMPT_WIDTH + self.line.char_cursor(), cols, out)
        } else {
            Ok(())
        }
    }

    /// Leva o cursor da tela até `cell`, só com movimento relativo.
    fn move_to(&mut self, cell: usize, cols: usize, out: &mut dyn fmt::Write) -> fmt::Result {
        let (from_row, from_col) = (self.at / cols, self.at % cols);
        let (to_row, to_col) = (cell / cols, cell % cols);
        if from_row == to_row {
            match to_col.cmp(&from_col) {
                Ordering::Greater => move_cursor(out, to_col - from_col, 'C')?,
                Ordering::Less => move_cursor(out, from_col - to_col, 'D')?,
                Ordering::Equal => {}
            }
        } else {
            if to_row < from_row {
                move_cursor(out, from_row - to_row, 'A')?;
            } else {
                move_cursor(out, to_row - from_row, 'B')?;
            }
            out.write_char('\r')?;
            move_cursor(out, to_col, 'C')?;
        }
        self.at = cell;
        Ok(())
    }

    /// Reescreve prompt e linha desde o início do prompt e põe o cursor onde a linha o tem.
    fn redraw(&mut self, cols: usize, out: &mut dyn fmt::Write) -> fmt::Result {
        move_cursor(out, self.at / cols, 'A')?;
        out.write_char('\r')?;
        out.write_str(PROMPT)?;
        out.write_str(self.line.as_str())?;
        let end = PROMPT_WIDTH + self.line.char_len();
        if end.is_multiple_of(cols) {
            out.write_str(WRAP_FIX)?;
        }
        out.write_str("\x1b[J")?;
        self.at = end;
        self.move_to(PROMPT_WIDTH + self.line.char_cursor(), cols, out)
    }

    fn move_to_end(&mut self, cols: usize, out: &mut dyn fmt::Write) -> fmt::Result {
        self.move_to(PROMPT_WIDTH + self.line.char_len(), cols, out)
    }

    /// Vai ao fim da linha e desce, deixando o cursor na coluna 0 de uma linha livre.
    fn finish_line(&mut self, cols: usize, out: &mut dyn fmt::Write) -> fmt::Result {
        self.move_to_end(cols, out)?;
        out.write_str(if self.at.is_multiple_of(cols) {
            "\r"
        } else {
            "\r\n"
        })?;
        self.at = 0;
        Ok(())
    }

    fn insert(&mut self, c: char, cols: usize, out: &mut dyn fmt::Write) -> fmt::Result {
        let at_end = self.line.cursor() == self.line.len();
        if !self.line.insert(c) {
            return out.write_char(BEL);
        }
        if !at_end {
            return self.redraw(cols, out);
        }
        out.write_char(c)?;
        self.at += 1;
        if self.at.is_multiple_of(cols) {
            out.write_str(WRAP_FIX)?;
        }
        Ok(())
    }

    fn backspace(&mut self, cols: usize, out: &mut dyn fmt::Write) -> fmt::Result {
        let at_end = self.line.cursor() == self.line.len();
        if !self.line.backspace() {
            return Ok(());
        }
        if at_end && !self.at.is_multiple_of(cols) {
            self.at -= 1;
            out.write_str("\x1b[D\x1b[K")
        } else {
            self.redraw(cols, out)
        }
    }

    fn submit(&mut self, host: &dyn Host, cols: usize, out: &mut dyn fmt::Write) -> fmt::Result {
        self.finish_line(cols, out)?;
        let len = self.line.len();
        let mut work = [0u8; LINE];
        work[..len].copy_from_slice(self.line.as_str().as_bytes());
        self.history.push(self.line.as_str());
        self.reset_line();
        self.run(&mut work[..len], host, out)?;
        self.redraw(cols, out)
    }

    fn run(&self, buf: &mut [u8], host: &dyn Host, out: &mut dyn fmt::Write) -> fmt::Result {
        let Some(args) = parse::tokenize(buf) else {
            return out.write_str("rustos: too many arguments\r\n");
        };
        let Some((name, rest)) = args.as_slice().split_first() else {
            return Ok(());
        };
        let Some(command) = commands::find(name) else {
            return write!(out, "rustos: command not found: {name}\r\n");
        };
        let mut ctx = Ctx {
            args: rest,
            host,
            out,
            history: &self.history,
        };
        (command.run)(&mut ctx)
    }

    fn recall_older(&mut self, cols: usize, out: &mut dyn fmt::Write) -> fmt::Result {
        let age = self.recall.map_or(0, |age| age + 1);
        let Some(entry) = self.history.get(age) else {
            return Ok(());
        };
        if self.recall.is_none() {
            self.draft.set(self.line.as_str());
        }
        self.line.set(entry);
        self.recall = Some(age);
        self.redraw(cols, out)
    }

    fn recall_newer(&mut self, cols: usize, out: &mut dyn fmt::Write) -> fmt::Result {
        match self.recall {
            None => return Ok(()),
            Some(0) => {
                self.line.set(self.draft.as_str());
                self.recall = None;
            }
            Some(age) => {
                self.line.set(self.history.get(age - 1).unwrap_or(""));
                self.recall = Some(age - 1);
            }
        }
        self.redraw(cols, out)
    }

    /// Tab: completa o nome do comando na primeira palavra; ambíguo, lista as opções.
    fn complete(&mut self, cols: usize, out: &mut dyn fmt::Write) -> fmt::Result {
        let typed = self.line.before_cursor();
        if typed.contains([' ', '\t', '\'', '"']) {
            return Ok(());
        }
        let (count, first, common) = analyze(typed, COMMANDS.iter().map(|c| c.name));
        if count == 0 {
            return out.write_char(BEL);
        }
        let prefix = &first[..common];
        let rest = &prefix[typed.len()..];
        let space = count == 1 && !self.line.after_cursor().starts_with(' ');
        if !rest.is_empty() || space {
            if self.line.free() < rest.len() + usize::from(space) {
                return out.write_char(BEL);
            }
            self.line.insert_str(rest);
            if space {
                self.line.insert(' ');
            }
            self.redraw(cols, out)?;
        }
        if count == 1 {
            Ok(())
        } else {
            self.list(prefix, cols, out)
        }
    }

    /// Lista, em colunas, os comandos que começam por `prefix` e redesenha o prompt.
    fn list(&mut self, prefix: &str, cols: usize, out: &mut dyn fmt::Write) -> fmt::Result {
        self.finish_line(cols, out)?;
        let names = || {
            COMMANDS
                .iter()
                .map(|c| c.name)
                .filter(|name| name.starts_with(prefix))
        };
        let width = names().map(str::len).max().unwrap_or(0) + 2;
        let per_row = (cols / width).max(1);
        let mut printed = 0;
        for name in names() {
            write!(out, "{name:<width$}")?;
            printed += 1;
            if printed % per_row == 0 {
                out.write_str("\r\n")?;
            }
        }
        if printed % per_row != 0 {
            out.write_str("\r\n")?;
        }
        self.redraw(cols, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Entries;
    use crate::testing::{FakeHost, Screen};

    type Sh = Shell<64, 4>;

    struct Term<const L: usize = 64, const H: usize = 4> {
        shell: Shell<L, H>,
        host: FakeHost,
        screen: Screen,
        top: usize,
    }

    impl<const L: usize, const H: usize> Term<L, H> {
        fn sized(cols: usize, rows: usize, deferred: bool) -> Self {
            let mut host = FakeHost::new();
            host.size = (u16::try_from(cols).unwrap(), u16::try_from(rows).unwrap());
            let mut term = Self {
                shell: Shell::new(),
                host,
                screen: Screen::new(cols, rows, deferred),
                top: 0,
            };
            term.shell.start(&term.host, &mut term.screen).unwrap();
            term.top = term.screen.cursor().0;
            term
        }

        fn new() -> Self {
            Self::sized(80, 100, true)
        }

        fn send(&mut self, bytes: &[u8]) {
            for &byte in bytes {
                self.shell
                    .feed(byte, &mut self.host, &mut self.screen)
                    .unwrap();
            }
        }

        fn type_str(&mut self, text: &str) {
            self.send(text.as_bytes());
        }

        /// Saída desde a última chamada.
        fn take_raw(&mut self) -> String {
            core::mem::take(&mut self.screen.raw)
        }

        fn line(&self) -> &str {
            self.shell.line.as_str()
        }

        /// A tela a partir da linha do primeiro prompt.
        fn lines(&self) -> Vec<String> {
            self.screen
                .text()
                .lines()
                .skip(self.top)
                .map(str::to_string)
                .collect()
        }
    }

    fn prompt_line(text: &str) -> String {
        format!("rustos ~ % {text}").trim_end().to_string()
    }

    #[test]
    fn start_prints_banner_and_prompt() {
        let term = Term::<64, 4>::new();
        let banner = term.screen.row(0);
        assert!(banner.starts_with("RustOS 0.1.0 (x86_64)"), "{banner}");
        assert!(term.screen.row(1).contains("'help'"));
        assert_eq!(term.lines(), [prompt_line("")]);
        assert_eq!(term.screen.cursor(), (2, PROMPT_WIDTH));
    }

    #[test]
    fn prompt_width_is_its_visible_width() {
        let mut visible = 0;
        let mut in_escape = false;
        for c in PROMPT.chars() {
            match (in_escape, c) {
                (_, '\x1b') => in_escape = true,
                (true, 'm') => in_escape = false,
                (true, _) => {}
                (false, _) => visible += 1,
            }
        }
        assert_eq!(visible, PROMPT_WIDTH);
    }

    #[test]
    fn start_again_resets_the_line() {
        let mut term = Term::<64, 4>::new();
        term.type_str("abc\x1b[");
        term.shell.start(&term.host, &mut term.screen).unwrap();
        assert_eq!(term.line(), "");
        term.type_str("A");
        assert_eq!(term.line(), "A");
    }

    #[test]
    fn typing_echoes_the_character_alone() {
        let mut term = Term::<64, 4>::new();
        term.take_raw();
        term.type_str("a");
        assert_eq!(term.take_raw(), "a");
        term.type_str("é");
        assert_eq!(term.take_raw(), "é");
    }

    #[test]
    fn enter_runs_the_command_and_shows_a_new_prompt() {
        let mut term = Term::<64, 4>::new();
        term.type_str("echo hi\r");
        assert_eq!(term.lines(), ["rustos ~ % echo hi", "hi", "rustos ~ %"]);
        assert_eq!(term.line(), "");
    }

    #[test]
    fn lf_alone_also_runs_the_command() {
        let mut term = Term::<64, 4>::new();
        term.type_str("echo hi\n");
        assert_eq!(term.lines(), ["rustos ~ % echo hi", "hi", "rustos ~ %"]);
    }

    #[test]
    fn cr_lf_runs_the_command_once() {
        let mut term = Term::<64, 4>::new();
        term.type_str("echo hi\r\n");
        assert_eq!(term.lines(), ["rustos ~ % echo hi", "hi", "rustos ~ %"]);
    }

    #[test]
    fn empty_line_only_shows_a_new_prompt() {
        let mut term = Term::<64, 4>::new();
        term.type_str("\r  \r");
        assert_eq!(
            term.lines(),
            [prompt_line(""), prompt_line(""), prompt_line("")]
        );
        assert_eq!(term.shell.history.count(), 0);
    }

    #[test]
    fn unknown_command_is_reported() {
        let mut term = Term::<64, 4>::new();
        term.type_str("foo bar\r");
        assert_eq!(term.lines()[1], "rustos: command not found: foo");
    }

    #[test]
    fn quotes_group_arguments() {
        let mut term = Term::<64, 4>::new();
        term.type_str("echo \"a  b\" 'c d' e\"f g\"\r");
        assert_eq!(term.lines()[1], "a  b c d ef g");
    }

    #[test]
    fn too_many_arguments_is_reported() {
        let mut term = Term::<64, 4>::new();
        term.type_str(&format!("echo {}\r", "x ".repeat(20)));
        assert_eq!(term.lines()[1], "rustos: too many arguments");
    }

    #[test]
    fn backspace_at_the_end_erases_in_place() {
        let mut term = Term::<64, 4>::new();
        term.type_str("ab");
        term.take_raw();
        term.send(&[0x7f]);
        assert_eq!(term.take_raw(), "\x1b[D\x1b[K");
        assert_eq!(term.line(), "a");
        term.send(&[0x08]);
        assert_eq!(term.lines(), [prompt_line("")]);
    }

    #[test]
    fn backspace_on_an_empty_line_does_nothing() {
        let mut term = Term::<64, 4>::new();
        term.take_raw();
        term.send(&[0x7f]);
        assert_eq!(term.take_raw(), "");
    }

    #[test]
    fn left_and_right_move_the_cursor_and_insert_in_the_middle() {
        let mut term = Term::<64, 4>::new();
        term.type_str("ac");
        term.take_raw();
        term.send(b"\x1b[D");
        assert_eq!(term.take_raw(), "\x1b[D");
        term.type_str("b");
        assert_eq!(term.line(), "abc");
        assert_eq!(term.lines(), [prompt_line("abc")]);
        assert_eq!(term.screen.cursor(), (2, PROMPT_WIDTH + 2));
        term.send(b"\x1b[C");
        assert_eq!(term.screen.cursor(), (2, PROMPT_WIDTH + 3));
    }

    #[test]
    fn arrows_at_the_edges_print_nothing() {
        let mut term = Term::<64, 4>::new();
        term.type_str("a");
        term.take_raw();
        term.send(b"\x1b[C");
        assert_eq!(term.take_raw(), "");
        term.send(b"\x1b[H");
        term.take_raw();
        term.send(b"\x1b[D");
        assert_eq!(term.take_raw(), "");
    }

    #[test]
    fn application_mode_arrows_work() {
        let mut term = Term::<64, 4>::new();
        term.type_str("ab\x1bOD\x1bODX");
        assert_eq!(term.line(), "Xab");
        term.type_str("\x1bOCY");
        assert_eq!(term.line(), "XaYb");
    }

    #[test]
    fn home_and_end_in_every_form() {
        for (home, end) in [
            ("\x1b[H", "\x1b[F"),
            ("\x1b[1~", "\x1b[4~"),
            ("\x1bOH", "\x1bOF"),
            ("\x01", "\x05"),
        ] {
            let mut term = Term::<64, 4>::new();
            term.type_str("bc");
            term.type_str(home);
            term.type_str("a");
            assert_eq!(term.screen.cursor(), (2, PROMPT_WIDTH + 1), "{home:?}");
            term.type_str(end);
            term.type_str("d");
            assert_eq!(term.line(), "abcd", "{end:?}");
            assert_eq!(term.lines(), [prompt_line("abcd")]);
        }
    }

    #[test]
    fn control_b_and_f_step_one_char() {
        let mut term = Term::<64, 4>::new();
        term.type_str("ab\x02\x02");
        assert_eq!(term.screen.cursor(), (2, PROMPT_WIDTH));
        term.type_str("\x06");
        assert_eq!(term.screen.cursor(), (2, PROMPT_WIDTH + 1));
    }

    #[test]
    fn delete_removes_under_the_cursor() {
        let mut term = Term::<64, 4>::new();
        term.type_str("abc\x1b[D\x1b[D\x1b[3~");
        assert_eq!(term.line(), "ac");
        assert_eq!(term.lines(), [prompt_line("ac")]);
        term.type_str("\x1b[F\x1b[3~");
        assert_eq!(term.line(), "ac");
    }

    #[test]
    fn control_k_kills_to_the_end() {
        let mut term = Term::<64, 4>::new();
        term.type_str("hello world\x01");
        term.type_str("\x06\x06\x06\x06\x06\x0b");
        assert_eq!(term.line(), "hello");
        assert_eq!(term.lines(), [prompt_line("hello")]);
        term.take_raw();
        term.type_str("\x0b");
        assert_eq!(term.take_raw(), "");
    }

    #[test]
    fn control_u_kills_to_the_start() {
        let mut term = Term::<64, 4>::new();
        term.type_str("hello world\x02\x02\x02\x02\x02\x15");
        assert_eq!(term.line(), "world");
        assert_eq!(term.lines(), [prompt_line("world")]);
        assert_eq!(term.screen.cursor(), (2, PROMPT_WIDTH));
    }

    #[test]
    fn control_w_kills_the_word_before_the_cursor() {
        let mut term = Term::<64, 4>::new();
        term.type_str("echo foo bar\x17");
        assert_eq!(term.line(), "echo foo ");
        term.type_str("\x17\x17\x17");
        assert_eq!(term.line(), "");
        assert_eq!(term.lines(), [prompt_line("")]);
    }

    #[test]
    fn control_l_clears_and_redraws_prompt_and_line() {
        let mut term = Term::<64, 4>::new();
        term.type_str("echo a\rabc\x02");
        term.type_str("\x0c");
        assert_eq!(term.screen.text(), "rustos ~ % abc");
        assert_eq!(term.screen.cursor(), (0, PROMPT_WIDTH + 2));
        assert!(term.screen.raw.contains("\x1b[H\x1b[2J"));
    }

    #[test]
    fn control_c_discards_the_line() {
        let mut term = Term::<64, 4>::new();
        term.type_str("abc\x03");
        assert_eq!(term.lines(), ["rustos ~ % abc^C", "rustos ~ %"]);
        assert_eq!(term.line(), "");
        assert_eq!(term.shell.history.count(), 0);
        term.type_str("echo x\r");
        assert_eq!(term.lines()[1], "rustos ~ % echo x");
    }

    #[test]
    fn control_c_with_the_cursor_in_the_middle_still_ends_the_line() {
        let mut term = Term::<64, 4>::new();
        term.type_str("abc\x02\x02\x03");
        assert_eq!(term.lines(), ["rustos ~ % abc^C", "rustos ~ %"]);
    }

    #[test]
    fn control_d_on_an_empty_line_does_nothing() {
        let mut term = Term::<64, 4>::new();
        term.take_raw();
        term.type_str("\x04");
        assert_eq!(term.take_raw(), "");
    }

    #[test]
    fn control_d_on_a_line_deletes_forward() {
        let mut term = Term::<64, 4>::new();
        term.type_str("abc\x01\x04");
        assert_eq!(term.line(), "bc");
    }

    #[test]
    fn unknown_sequences_leave_no_garbage() {
        let mut term = Term::<64, 4>::new();
        term.type_str("a\x1b[99~\x1b[1;5C\x1b[200~\x1bx\x1bOP\x1b[?1;2c\x1b[2Jb");
        assert_eq!(term.line(), "ab");
    }

    #[test]
    fn full_line_ignores_input_and_rings_the_bell() {
        let mut term = Term::<4, 4>::new();
        term.type_str("abcd");
        assert_eq!(term.screen.bells, 0);
        term.type_str("ef");
        assert_eq!(term.line(), "abcd");
        assert_eq!(term.screen.bells, 2);
        assert_eq!(term.lines(), [prompt_line("abcd")]);
    }

    #[test]
    fn full_line_still_accepts_edits() {
        let mut term = Term::<4, 4>::new();
        term.type_str("abcd\x7fX");
        assert_eq!(term.line(), "abcX");
        assert_eq!(term.screen.bells, 0);
    }

    #[test]
    fn multibyte_chars_edit_and_move_by_character() {
        let mut term = Term::<64, 4>::new();
        term.type_str("aé€😀");
        assert_eq!(term.screen.cursor(), (2, PROMPT_WIDTH + 4));
        term.type_str("\x1b[D\x1b[D");
        assert_eq!(term.screen.cursor(), (2, PROMPT_WIDTH + 2));
        term.type_str("X");
        assert_eq!(term.line(), "aéX€😀");
        term.type_str("\x7f\x7f");
        assert_eq!(term.line(), "a€😀");
        assert_eq!(term.lines(), [prompt_line("a€😀")]);
        term.type_str("\x1b[3~");
        assert_eq!(term.line(), "a😀");
        assert_eq!(term.screen.cursor(), (2, PROMPT_WIDTH + 1));
    }

    #[test]
    fn multibyte_char_arriving_byte_by_byte_is_echoed_whole() {
        let mut term = Term::<64, 4>::new();
        term.take_raw();
        term.send(&[0xe2, 0x82]);
        assert_eq!(term.take_raw(), "");
        term.send(&[0xac]);
        assert_eq!(term.take_raw(), "€");
    }

    #[test]
    fn up_and_down_walk_the_history_and_keep_the_draft() {
        let mut term = Term::<64, 4>::new();
        term.type_str("echo one\recho two\rdraft");
        term.type_str("\x1b[A");
        assert_eq!(term.line(), "echo two");
        term.type_str("\x1b[A");
        assert_eq!(term.line(), "echo one");
        term.type_str("\x1b[A");
        assert_eq!(term.line(), "echo one");
        term.type_str("\x1b[B");
        assert_eq!(term.line(), "echo two");
        term.type_str("\x1b[B");
        assert_eq!(term.line(), "draft");
        term.type_str("\x1b[B");
        assert_eq!(term.line(), "draft");
        assert_eq!(term.lines().last().unwrap(), "rustos ~ % draft");
    }

    #[test]
    fn control_p_and_n_also_walk_the_history() {
        let mut term = Term::<64, 4>::new();
        term.type_str("echo one\r\x10");
        assert_eq!(term.line(), "echo one");
        term.type_str("\x0e");
        assert_eq!(term.line(), "");
    }

    #[test]
    fn history_recall_runs_again() {
        let mut term = Term::<64, 4>::new();
        term.type_str("echo one\r\x1b[A\r");
        assert_eq!(term.lines()[2], "rustos ~ % echo one");
        assert_eq!(term.lines()[3], "one");
        assert_eq!(term.shell.history.count(), 1);
    }

    #[test]
    fn up_with_no_history_does_nothing() {
        let mut term = Term::<64, 4>::new();
        term.type_str("ab");
        term.take_raw();
        term.type_str("\x1b[A\x1b[B");
        assert_eq!(term.take_raw(), "");
        assert_eq!(term.line(), "ab");
    }

    #[test]
    fn history_drops_duplicates_blanks_and_the_oldest() {
        let mut term = Term::<64, 2>::new();
        term.type_str("echo a\recho a\r \recho b\recho c\r");
        assert_eq!(term.shell.history.count(), 2);
        term.type_str("\x1b[A\x1b[A\x1b[A");
        assert_eq!(term.line(), "echo b");
    }

    #[test]
    fn history_command_lists_numbered_lines() {
        let mut term = Term::<64, 4>::new();
        term.type_str("echo a\rhistory\r");
        let lines = term.lines();
        assert_eq!(lines[2], "rustos ~ % history");
        assert_eq!(lines[3], "    1  echo a");
        assert_eq!(lines[4], "    2  history");
    }

    #[test]
    fn zero_history_shell_still_works() {
        let mut term = Term::<64, 0>::new();
        term.type_str("echo a\r\x1b[A");
        assert_eq!(term.line(), "");
        assert_eq!(term.lines()[1], "a");
    }

    #[test]
    fn tab_completes_a_unique_command() {
        let mut term = Term::<64, 4>::new();
        term.type_str("ver\t");
        assert_eq!(term.line(), "version ");
        assert_eq!(term.lines(), [prompt_line("version")]);
        assert_eq!(term.screen.cursor(), (2, PROMPT_WIDTH + 8));
    }

    #[test]
    fn tab_completed_command_runs() {
        let mut term = Term::<64, 4>::new();
        term.type_str("vers\t\r");
        assert_eq!(term.lines()[1], "RustOS 0.1.0");
    }

    #[test]
    fn tab_lists_ambiguous_options_and_keeps_the_line() {
        let mut term = Term::<64, 4>::new();
        term.type_str("h\t");
        assert_eq!(term.line(), "h");
        assert_eq!(
            term.lines(),
            ["rustos ~ % h", "help     history", "rustos ~ % h"]
        );
        assert_eq!(term.screen.cursor(), (4, PROMPT_WIDTH + 1));
    }

    #[test]
    fn tab_on_an_empty_line_lists_every_command() {
        let mut term = Term::<64, 4>::new();
        term.type_str("\t");
        let text = term.screen.text();
        for command in COMMANDS {
            assert!(text.contains(command.name), "{}", command.name);
        }
        assert_eq!(term.line(), "");
        assert_eq!(term.lines().last().unwrap(), "rustos ~ %");
    }

    #[test]
    fn tab_listing_wraps_to_the_terminal_width() {
        let mut term = Term::<64, 4>::sized(20, 40, true);
        term.type_str("\t");
        let rows = term.lines();
        assert!(rows.len() > 3, "{rows:?}");
        assert!(rows.iter().all(|row| row.chars().count() <= 20));
        assert_eq!(rows.last().unwrap(), "rustos ~ %");
    }

    #[test]
    fn tab_with_no_match_rings_the_bell() {
        let mut term = Term::<64, 4>::new();
        term.type_str("zzz\t");
        assert_eq!(term.line(), "zzz");
        assert_eq!(term.screen.bells, 1);
    }

    #[test]
    fn tab_after_the_first_word_does_nothing() {
        let mut term = Term::<64, 4>::new();
        term.type_str("echo ve\t");
        assert_eq!(term.line(), "echo ve");
        assert_eq!(term.screen.bells, 0);
        term.type_str("\x15\"ve\t");
        assert_eq!(term.line(), "\"ve");
    }

    #[test]
    fn tab_does_not_add_a_space_before_one_that_exists() {
        let mut term = Term::<64, 4>::new();
        term.type_str("ver x\x02\x02\t");
        assert_eq!(term.line(), "version x");
    }

    #[test]
    fn tab_without_room_rings_the_bell() {
        let mut term = Term::<6, 4>::new();
        term.type_str("hist\t");
        assert_eq!(term.line(), "hist");
        assert_eq!(term.screen.bells, 1);
    }

    #[test]
    fn analyze_finds_the_common_prefix() {
        let names = ["status", "stash", "stop", "uname"];
        assert_eq!(analyze("sta", names.into_iter()), (2, "status", 3));
        assert_eq!(analyze("s", names.into_iter()), (3, "status", 2));
        assert_eq!(analyze("stat", names.into_iter()), (1, "status", 6));
        assert_eq!(analyze("x", names.into_iter()).0, 0);
        assert_eq!(analyze("", names.into_iter()), (4, "status", 0));
    }

    #[test]
    fn move_cursor_omits_a_count_of_one_and_skips_zero() {
        let mut out = String::new();
        move_cursor(&mut out, 0, 'A').unwrap();
        move_cursor(&mut out, 1, 'B').unwrap();
        move_cursor(&mut out, 12, 'C').unwrap();
        assert_eq!(out, "\x1b[B\x1b[12C");
    }

    #[test]
    fn zero_columns_falls_back_to_eighty() {
        let mut host = FakeHost::new();
        host.size = (0, 0);
        assert_eq!(terminal_cols(&host), 80);
        host.size = (132, 50);
        assert_eq!(terminal_cols(&host), 132);
    }

    #[test]
    fn commands_work_through_the_keyboard() {
        let mut term = Term::<64, 4>::new();
        term.type_str("uname -a\r");
        assert_eq!(term.lines()[1], "RustOS 0.1.0 x86_64");
        term.type_str("uptime\r");
        assert_eq!(term.lines()[3], "up 1m 05s");
        term.host.uptime = None;
        term.type_str("uptime\r");
        assert!(term.screen.text().contains("not available yet"));
    }

    #[test]
    fn fetch_lays_the_logo_beside_the_labels() {
        let mut term = Term::<64, 4>::new();
        term.type_str("fetch\r");
        let lines = term.lines();
        assert!(lines[1].starts_with("  .-----------.  "), "{}", lines[1]);
        assert!(lines[1].ends_with("OS: RustOS"), "{}", lines[1]);
        assert!(lines[9].contains("Uptime: 1m 05s"), "{}", lines[9]);
        assert_eq!(lines.last().unwrap(), "rustos ~ %");
    }

    #[test]
    fn mem_and_colors_and_help_run() {
        let mut term = Term::<64, 4>::new();
        term.type_str("mem\rcolors\rhelp\r");
        let text = term.screen.text();
        assert!(text.contains("frame meta"));
        assert!(text.contains("Grayscale (232-255)"));
        assert!(text.contains("list the commands"));
    }

    #[test]
    fn clear_command_leaves_only_the_prompt() {
        let mut term = Term::<64, 4>::new();
        term.type_str("echo a\rclear\r");
        assert_eq!(term.screen.text(), "rustos ~ %");
        assert_eq!(term.screen.cursor(), (0, PROMPT_WIDTH));
    }

    #[test]
    fn shell_fits_in_a_static_and_in_a_few_kib() {
        static SHELL: Shell<256, 32> = Shell::new();
        assert!(size_of_val(&SHELL) < 10 * 1024);
        let _ = Sh::default();
    }

    /// Prompt, linha e cursor esperados na tela, com a linha partida a cada `cols` células.
    fn assert_screen_matches<const L: usize, const H: usize>(term: &Term<L, H>, cols: usize) {
        let text: Vec<char> = format!("rustos ~ % {}", term.line()).chars().collect();
        let rows: Vec<String> = text
            .chunks(cols)
            .map(|c| c.iter().collect::<String>())
            .collect();
        let first = term.top;
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(
                term.screen.row(first + index).trim_end(),
                row.trim_end(),
                "row {index}"
            );
        }
        for y in first + rows.len()..first + rows.len() + 2 {
            assert_eq!(term.screen.row(y), "", "stale row {y}");
        }
        let cell = PROMPT_WIDTH + term.line()[..term.shell.line.cursor()].chars().count();
        assert_eq!(
            term.screen.cursor(),
            (first + cell / cols, cell % cols),
            "line {:?}",
            term.line()
        );
    }

    #[test]
    fn long_lines_wrap_without_corrupting_the_screen() {
        for deferred in [true, false] {
            let mut term = Term::<64, 4>::sized(20, 30, deferred);
            for step in 0..40 {
                term.type_str("abcdefghij".get(step % 10..=step % 10).unwrap());
                assert_screen_matches(&term, 20);
            }
        }
    }

    #[test]
    fn a_line_ending_exactly_at_the_last_column_puts_the_cursor_on_the_next_row() {
        for deferred in [true, false] {
            let mut term = Term::<64, 4>::sized(20, 30, deferred);
            term.type_str("123456789");
            let top = term.top;
            assert_eq!(term.screen.cursor(), (top + 1, 0), "deferred {deferred}");
            assert_eq!(term.screen.row(top), "rustos ~ % 123456789");
            term.type_str("x");
            assert_eq!(term.screen.row(top + 1), "x");
            term.type_str("\x7f\x7f");
            assert_eq!(term.screen.cursor(), (top, 19));
            assert_screen_matches(&term, 20);
        }
    }

    #[test]
    fn enter_after_an_exact_fit_does_not_leave_a_blank_row() {
        let mut term = Term::<64, 4>::sized(20, 30, true);
        term.type_str("123456789\r");
        let top = term.top;
        assert_eq!(term.screen.row(top), "rustos ~ % 123456789");
        assert_eq!(term.screen.row(top + 1), "rustos: command not");
    }

    #[test]
    fn editing_inside_a_wrapped_line_redraws_every_row() {
        for deferred in [true, false] {
            let mut term = Term::<64, 4>::sized(20, 30, deferred);
            term.type_str("the quick brown fox jumps over the lazy dog");
            assert_screen_matches(&term, 20);
            term.type_str("\x01");
            assert_screen_matches(&term, 20);
            term.type_str("X\x1b[3~\x1b[3~");
            assert_screen_matches(&term, 20);
            term.type_str(
                "\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C\x1b[C",
            );
            assert_screen_matches(&term, 20);
            term.type_str("\x7f\x17\x0b");
            assert_screen_matches(&term, 20);
            term.type_str("\x15");
            assert_screen_matches(&term, 20);
        }
    }

    #[test]
    fn history_recall_replaces_long_lines_with_short_ones() {
        let mut term = Term::<64, 4>::sized(20, 40, true);
        term.type_str("echo hi\recho the quick brown fox jumps over\r");
        term.top = term.screen.cursor().0;
        term.type_str("\x1b[A");
        assert_screen_matches(&term, 20);
        term.type_str("\x1b[A");
        assert_screen_matches(&term, 20);
        term.type_str("\x1b[B\x1b[B");
        assert_screen_matches(&term, 20);
    }

    #[test]
    fn a_wrapped_line_at_the_bottom_scrolls_and_keeps_the_prompt_intact() {
        for deferred in [true, false] {
            let mut term = Term::<64, 4>::sized(20, 4, deferred);
            term.type_str("the quick brown fox jumps over the lazy dog");
            let rows = term.screen.text();
            assert!(rows.ends_with("r the lazy dog"), "{rows}");
            assert!(rows.contains("rustos ~ % the quick"), "{rows}");
            term.type_str("\x01Z\x05\x15");
            assert!(
                term.screen.text().ends_with("rustos ~ %"),
                "{}",
                term.screen.text()
            );
        }
    }

    #[test]
    fn enter_on_a_wrapped_line_runs_it_and_prints_below() {
        let mut term = Term::<64, 4>::sized(20, 30, true);
        term.type_str("echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\x01\r");
        let lines = term.lines();
        assert_eq!(lines[3], "a".repeat(20));
        assert_eq!(lines[4], "a".repeat(10));
        assert_eq!(lines.last().unwrap(), "rustos ~ %");
    }

    #[test]
    fn random_editing_keeps_the_screen_in_sync_with_the_line() {
        let alphabet = [
            "a", "é", "€", " ", "\x7f", "\x1b[D", "\x1b[C", "\x01", "\x05", "\x0b", "\x15", "\x17",
            "\x1b[3~",
        ];
        for deferred in [true, false] {
            for seed in 0..12u64 {
                let mut state = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
                let mut term = Term::<48, 4>::sized(17, 60, deferred);
                for _ in 0..400 {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    let key = alphabet[usize::try_from(state >> 33).unwrap() % alphabet.len()];
                    let key = if state & 0x700 == 0 { alphabet[0] } else { key };
                    term.type_str(key);
                    assert_screen_matches(&term, 17);
                }
            }
        }
    }

    /// Quantas linhas de `text` (cada uma terminada em `\n`) ocupam a tela, com `cols` colunas.
    fn rows_of(text: &str, cols: usize) -> usize {
        text.lines().map(|l| l.len().div_ceil(cols).max(1)).sum()
    }

    /// Faz o que o kernel faz com a saída de segundo plano: guarda, e o shell a mostra.
    fn interject<const L: usize, const H: usize, const N: usize>(
        term: &mut Term<L, H>,
        backlog: &mut Backlog<N>,
        cols: usize,
    ) -> usize {
        let (row, _) = term.screen.cursor();
        let above = term.shell.at / cols;
        let first = row - above;
        let mut text = String::new();
        let mut copy = backlog.clone();
        copy.drain(&mut text).unwrap();
        term.shell
            .interject(&term.host, backlog, &mut term.screen)
            .unwrap();
        first + rows_of(&text, cols)
    }

    fn onlcr_term<const L: usize>(cols: usize, rows: usize) -> Term<L, 4> {
        let mut term = Term::<L, 4>::sized(cols, rows, true);
        term.screen = Screen::new(cols, rows, true).with_onlcr();
        term.shell.start(&term.host, &mut term.screen).unwrap();
        term.top = term.screen.cursor().0;
        term
    }

    #[test]
    fn background_output_lands_above_the_prompt_and_keeps_the_line() {
        let mut term = onlcr_term::<64>(80, 30);
        term.type_str("ab\x1b[D");
        let mut backlog = Backlog::<256>::new();
        backlog.push(format_args!("[user] count: 1/5\n[user] count: 2/5\n"));
        let prompt = interject(&mut term, &mut backlog, 80);
        assert_eq!(
            term.lines(),
            ["[user] count: 1/5", "[user] count: 2/5", "rustos ~ % ab"]
        );
        assert_eq!(term.line(), "ab");
        term.top = prompt;
        assert_screen_matches(&term, 80);
        assert!(backlog.is_empty());
        term.type_str("c");
        assert_eq!(term.line(), "acb");
        assert_screen_matches(&term, 80);
    }

    #[test]
    fn background_output_never_sticks_to_the_prompt() {
        let mut term = onlcr_term::<64>(80, 30);
        let mut backlog = Backlog::<256>::new();
        for n in 1..=3 {
            backlog.push(format_args!("[user] count: {n}/3\n"));
            interject(&mut term, &mut backlog, 80);
            term.type_str("x");
        }
        assert_eq!(
            term.lines(),
            [
                "[user] count: 1/3",
                "[user] count: 2/3",
                "[user] count: 3/3",
                "rustos ~ % xxx"
            ]
        );
    }

    #[test]
    fn background_output_clears_every_row_of_a_wrapped_line() {
        for deferred in [true, false] {
            let mut term = Term::<64, 4>::sized(20, 30, deferred);
            term.screen = Screen::new(20, 30, deferred).with_onlcr();
            term.shell.start(&term.host, &mut term.screen).unwrap();
            term.top = term.screen.cursor().0;
            term.type_str("the quick brown fox jumps over\x1b[D\x1b[D");
            let mut backlog = Backlog::<256>::new();
            backlog.push(format_args!("[user] hi\n"));
            term.top = interject(&mut term, &mut backlog, 20);
            assert_eq!(term.screen.row(term.top - 1), "[user] hi");
            assert_screen_matches(&term, 20);
        }
    }

    #[test]
    fn an_empty_backlog_writes_nothing() {
        let mut term = Term::<64, 4>::new();
        term.type_str("ab");
        term.take_raw();
        let mut backlog = Backlog::<16>::new();
        term.shell
            .interject(&term.host, &mut backlog, &mut term.screen)
            .unwrap();
        assert_eq!(term.take_raw(), "");
    }

    #[test]
    fn a_full_backlog_says_what_it_lost_above_the_prompt() {
        let mut term = onlcr_term::<64>(80, 30);
        let mut backlog = Backlog::<16>::new();
        backlog.push(format_args!("[user] first\n"));
        backlog.push(format_args!("[user] second\n"));
        interject(&mut term, &mut backlog, 80);
        assert_eq!(
            term.lines(),
            [
                "[user] first",
                "[rustos] user output: 1 writes lost (backlog full)",
                "rustos ~ %"
            ]
        );
    }

    #[test]
    fn the_prompt_comes_back_exactly_where_it_was_after_a_command() {
        let mut term = onlcr_term::<64>(80, 30);
        term.type_str("echo hi\r");
        let mut backlog = Backlog::<64>::new();
        backlog.push(format_args!("[user] late\n"));
        interject(&mut term, &mut backlog, 80);
        assert_eq!(
            term.lines(),
            ["rustos ~ % echo hi", "hi", "[user] late", "rustos ~ %"]
        );
    }
}
