//! Máquina de estados do DEC ANSI parser (Paul Williams), com UTF-8 na frente.
//! Só reconhece: quem executa cada sequência é o `Terminal`.

pub(crate) const MAX_PARAMS: usize = 16;
const REPLACEMENT: char = '\u{FFFD}';
const MANY_INTERMEDIATES: u8 = 0xFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    OscString,
    StringIgnore,
}

/// O que um byte fez: nada, ou uma ação para o terminal executar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Event {
    None,
    Print(char),
    Execute(u8),
    Esc(u8),
    Csi(u8),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Parser {
    state: State,
    params: [u16; MAX_PARAMS],
    nparams: usize,
    overflow: bool,
    marker: u8,
    inter: u8,
    utf8_acc: u32,
    utf8_need: u8,
    utf8_min: u32,
    replay: Option<u8>,
}

impl Parser {
    pub(crate) const fn new() -> Self {
        Parser {
            state: State::Ground,
            params: [0; MAX_PARAMS],
            nparams: 0,
            overflow: false,
            marker: 0,
            inter: 0,
            utf8_acc: 0,
            utf8_need: 0,
            utf8_min: 0,
            replay: None,
        }
    }

    /// Parado no texto: sem sequência aberta e sem UTF-8 pela metade.
    pub(crate) fn is_idle(&self) -> bool {
        self.state == State::Ground && self.utf8_need == 0
    }

    /// Parâmetro `index`; 0 se omitido ou fora da lista. Valores saturam em 65535.
    pub(crate) fn param(&self, index: usize) -> u16 {
        if index < self.nparams {
            self.params[index]
        } else {
            0
        }
    }

    pub(crate) fn param_count(&self) -> usize {
        self.nparams
    }

    /// Marcador privado (`?`, `<`, `=`, `>`) ou 0.
    pub(crate) fn marker(&self) -> u8 {
        self.marker
    }

    /// Byte intermediário ou 0; `0xFF` se vieram vários.
    pub(crate) fn intermediate(&self) -> u8 {
        self.inter
    }

    /// Byte que abortou um UTF-8 e precisa ser processado de novo.
    pub(crate) fn take_replay(&mut self) -> Option<u8> {
        self.replay.take()
    }

    pub(crate) fn advance(&mut self, byte: u8) -> Event {
        if self.utf8_need > 0 {
            return self.continue_utf8(byte);
        }
        if byte < 0x80 {
            return self.advance_ascii(byte);
        }
        let (need, acc, min) = match byte {
            0xC2..=0xDF => (1, u32::from(byte & 0x1F), 0x80),
            0xE0..=0xEF => (2, u32::from(byte & 0x0F), 0x800),
            0xF0..=0xF4 => (3, u32::from(byte & 0x07), 0x1_0000),
            _ => return self.text(REPLACEMENT),
        };
        self.utf8_need = need;
        self.utf8_acc = acc;
        self.utf8_min = min;
        Event::None
    }

    fn continue_utf8(&mut self, byte: u8) -> Event {
        if byte & 0xC0 != 0x80 {
            self.utf8_need = 0;
            self.replay = Some(byte);
            return self.text(REPLACEMENT);
        }
        self.utf8_acc = (self.utf8_acc << 6) | u32::from(byte & 0x3F);
        self.utf8_need -= 1;
        if self.utf8_need > 0 {
            return Event::None;
        }
        let decoded = if self.utf8_acc >= self.utf8_min {
            char::from_u32(self.utf8_acc)
        } else {
            None
        };
        self.text(decoded.unwrap_or(REPLACEMENT))
    }

    fn text(&self, c: char) -> Event {
        if self.state == State::Ground && !('\u{80}'..='\u{9F}').contains(&c) {
            Event::Print(c)
        } else {
            Event::None
        }
    }

    fn clear(&mut self) {
        self.params = [0; MAX_PARAMS];
        self.nparams = 0;
        self.overflow = false;
        self.marker = 0;
        self.inter = 0;
    }

    fn collect(&mut self, byte: u8) {
        self.inter = if self.inter == 0 {
            byte
        } else {
            MANY_INTERMEDIATES
        };
    }

    fn param_byte(&mut self, byte: u8) {
        if self.nparams == 0 {
            self.nparams = 1;
        }
        if byte == b';' {
            if self.nparams < MAX_PARAMS {
                self.nparams += 1;
            } else {
                self.overflow = true;
            }
        } else if !self.overflow {
            let slot = &mut self.params[self.nparams - 1];
            *slot = slot
                .saturating_mul(10)
                .saturating_add(u16::from(byte - b'0'));
        }
    }

    fn advance_ascii(&mut self, byte: u8) -> Event {
        match byte {
            0x18 | 0x1A => {
                self.state = State::Ground;
                Event::None
            }
            0x1B => {
                self.clear();
                self.state = State::Escape;
                Event::None
            }
            _ => match self.state {
                State::Ground => match byte {
                    0x00..=0x1F => Event::Execute(byte),
                    0x20..=0x7E => Event::Print(char::from(byte)),
                    _ => Event::None,
                },
                State::Escape => self.escape_byte(byte),
                State::EscapeIntermediate => self.escape_intermediate_byte(byte),
                State::OscString => {
                    if byte == 0x07 {
                        self.state = State::Ground;
                    }
                    Event::None
                }
                State::StringIgnore => Event::None,
                State::CsiEntry | State::CsiParam | State::CsiIntermediate | State::CsiIgnore => {
                    self.csi_byte(byte)
                }
            },
        }
    }

    fn escape_byte(&mut self, byte: u8) -> Event {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => Event::Execute(byte),
            0x20..=0x2F => {
                self.collect(byte);
                self.state = State::EscapeIntermediate;
                Event::None
            }
            b'[' => {
                self.state = State::CsiEntry;
                Event::None
            }
            b']' => {
                self.state = State::OscString;
                Event::None
            }
            b'P' | b'X' | b'^' | b'_' => {
                self.state = State::StringIgnore;
                Event::None
            }
            0x30..=0x7E => {
                self.state = State::Ground;
                Event::Esc(byte)
            }
            _ => Event::None,
        }
    }

    fn escape_intermediate_byte(&mut self, byte: u8) -> Event {
        match byte {
            0x00..=0x17 | 0x19 | 0x1C..=0x1F => Event::Execute(byte),
            0x20..=0x2F => {
                self.collect(byte);
                Event::None
            }
            0x30..=0x7E => {
                self.state = State::Ground;
                Event::Esc(byte)
            }
            _ => Event::None,
        }
    }

    fn csi_byte(&mut self, byte: u8) -> Event {
        match (self.state, byte) {
            (_, 0x00..=0x17 | 0x19 | 0x1C..=0x1F) => Event::Execute(byte),
            (_, 0x7F) | (State::CsiIgnore, 0x20..=0x3F) => Event::None,
            (State::CsiIgnore, _) => {
                self.state = State::Ground;
                Event::None
            }
            (_, 0x40..=0x7E) => {
                self.state = State::Ground;
                Event::Csi(byte)
            }
            (State::CsiIntermediate, 0x20..=0x2F) => {
                self.collect(byte);
                Event::None
            }
            (_, 0x20..=0x2F) => {
                self.collect(byte);
                self.state = State::CsiIntermediate;
                Event::None
            }
            (State::CsiEntry | State::CsiParam, b'0'..=b'9' | b';') => {
                self.param_byte(byte);
                self.state = State::CsiParam;
                Event::None
            }
            (State::CsiEntry, 0x3C..=0x3F) => {
                self.marker = byte;
                self.state = State::CsiParam;
                Event::None
            }
            _ => {
                self.state = State::CsiIgnore;
                Event::None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(parser: &mut Parser, bytes: &[u8]) -> Vec<Event> {
        let mut events = Vec::new();
        for &byte in bytes {
            let event = parser.advance(byte);
            if event != Event::None {
                events.push(event);
            }
            if let Some(again) = parser.take_replay() {
                let event = parser.advance(again);
                if event != Event::None {
                    events.push(event);
                }
            }
        }
        events
    }

    fn events(bytes: &[u8]) -> Vec<Event> {
        run(&mut Parser::new(), bytes)
    }

    #[test]
    fn printable_ascii_becomes_print_events() {
        assert_eq!(events(b"hi"), [Event::Print('h'), Event::Print('i')]);
    }

    #[test]
    fn c0_controls_execute_and_del_is_ignored() {
        assert_eq!(
            events(b"\n\r\x07\x7f"),
            [Event::Execute(10), Event::Execute(13), Event::Execute(7)]
        );
    }

    #[test]
    fn csi_collects_params_and_dispatches_on_the_final_byte() {
        let mut parser = Parser::new();
        assert_eq!(run(&mut parser, b"\x1b[12;34H"), [Event::Csi(b'H')]);
        assert_eq!(parser.param_count(), 2);
        assert_eq!((parser.param(0), parser.param(1)), (12, 34));
        assert_eq!(parser.param(2), 0);
        assert_eq!(parser.marker(), 0);
    }

    #[test]
    fn empty_params_read_as_zero_but_still_count() {
        let mut parser = Parser::new();
        run(&mut parser, b"\x1b[;5H");
        assert_eq!(parser.param_count(), 2);
        assert_eq!((parser.param(0), parser.param(1)), (0, 5));
        run(&mut parser, b"\x1b[H");
        assert_eq!(parser.param_count(), 0);
    }

    #[test]
    fn huge_params_saturate() {
        let mut parser = Parser::new();
        run(&mut parser, b"\x1b[99999999999999999999A");
        assert_eq!(parser.param(0), u16::MAX);
    }

    #[test]
    fn extra_params_are_dropped_without_corrupting_the_last_kept() {
        let mut parser = Parser::new();
        let mut seq = b"\x1b[".to_vec();
        for i in 1..=40 {
            seq.extend_from_slice(format!("{i};").as_bytes());
        }
        seq.push(b'm');
        assert_eq!(run(&mut parser, &seq), [Event::Csi(b'm')]);
        assert_eq!(parser.param_count(), MAX_PARAMS);
        assert_eq!(parser.param(MAX_PARAMS - 1), 16);
    }

    #[test]
    fn private_marker_is_recorded() {
        let mut parser = Parser::new();
        assert_eq!(run(&mut parser, b"\x1b[?25l"), [Event::Csi(b'l')]);
        assert_eq!((parser.marker(), parser.param(0)), (b'?', 25));
    }

    #[test]
    fn marker_after_params_aborts_the_sequence() {
        let mut parser = Parser::new();
        assert_eq!(run(&mut parser, b"\x1b[1?2mX"), [Event::Print('X')]);
    }

    #[test]
    fn colon_in_params_aborts_the_sequence() {
        assert_eq!(events(b"\x1b[38:2:1:2:3mX"), [Event::Print('X')]);
    }

    #[test]
    fn csi_intermediates_are_recorded() {
        let mut parser = Parser::new();
        assert_eq!(run(&mut parser, b"\x1b[2 q"), [Event::Csi(b'q')]);
        assert_eq!(parser.intermediate(), b' ');
        run(&mut parser, b"\x1b[ !p");
        assert_eq!(parser.intermediate(), MANY_INTERMEDIATES);
    }

    #[test]
    fn parameter_after_intermediate_ignores_the_sequence() {
        assert_eq!(events(b"\x1b[ 1mX"), [Event::Print('X')]);
    }

    #[test]
    fn c0_inside_csi_executes_without_aborting() {
        let mut parser = Parser::new();
        assert_eq!(
            run(&mut parser, b"\x1b[1\n2A"),
            [Event::Execute(10), Event::Csi(b'A')]
        );
        assert_eq!(parser.param(0), 12);
    }

    #[test]
    fn esc_inside_csi_restarts_the_sequence() {
        let mut parser = Parser::new();
        assert_eq!(run(&mut parser, b"\x1b[1;2\x1b[3A"), [Event::Csi(b'A')]);
        assert_eq!((parser.param_count(), parser.param(0)), (1, 3));
    }

    #[test]
    fn can_and_sub_cancel_a_sequence() {
        assert_eq!(events(b"\x1b[12\x18A"), [Event::Print('A')]);
        assert_eq!(events(b"\x1b[12\x1aA"), [Event::Print('A')]);
        assert_eq!(events(b"\x1b]0;t\x18A"), [Event::Print('A')]);
    }

    #[test]
    fn escape_sequences_dispatch_with_intermediates() {
        let mut parser = Parser::new();
        assert_eq!(run(&mut parser, b"\x1b7"), [Event::Esc(b'7')]);
        assert_eq!(parser.intermediate(), 0);
        assert_eq!(run(&mut parser, b"\x1b(B"), [Event::Esc(b'B')]);
        assert_eq!(parser.intermediate(), b'(');
    }

    #[test]
    fn osc_ends_at_bel_or_st_and_emits_nothing() {
        assert_eq!(events(b"\x1b]0;title\x07A"), [Event::Print('A')]);
        assert_eq!(
            events(b"\x1b]0;title\x1b\\A"),
            [Event::Esc(b'\\'), Event::Print('A')]
        );
    }

    #[test]
    fn osc_swallows_text_controls_and_csi_lookalikes() {
        assert_eq!(events(b"\x1b]2;a\nb[1mc\xc3\xa9\x07Z"), [Event::Print('Z')]);
    }

    #[test]
    fn dcs_sos_pm_apc_run_until_st_and_ignore_bel() {
        for intro in *b"PX^_" {
            let seq = [0x1B, intro, b'q', 0x07, b'x', 0x1B, b'\\', b'A'];
            assert_eq!(events(&seq), [Event::Esc(b'\\'), Event::Print('A')]);
        }
    }

    #[test]
    fn utf8_sequences_decode_to_one_print() {
        assert_eq!(
            events("é€😀".as_bytes()),
            [Event::Print('é'), Event::Print('€'), Event::Print('😀')]
        );
    }

    #[test]
    fn utf8_resumes_after_a_split() {
        let mut parser = Parser::new();
        let bytes = "€".as_bytes();
        assert_eq!(run(&mut parser, &bytes[..1]), []);
        assert!(!parser.is_idle());
        assert_eq!(run(&mut parser, &bytes[1..2]), []);
        assert_eq!(run(&mut parser, &bytes[2..]), [Event::Print('€')]);
        assert!(parser.is_idle());
    }

    #[test]
    fn invalid_utf8_becomes_replacement_characters() {
        let bad = Event::Print(REPLACEMENT);
        assert_eq!(events(&[0x80]), [bad]);
        assert_eq!(events(&[0xC0, 0xAF]), [bad, bad]);
        assert_eq!(events(&[0xFF]), [bad]);
        assert_eq!(events(&[0xF5, 0x80]), [bad, bad]);
    }

    #[test]
    fn truncated_utf8_is_replaced_and_the_next_byte_is_kept() {
        let bad = Event::Print(REPLACEMENT);
        assert_eq!(events(&[0xE2, 0x82, b'A']), [bad, Event::Print('A')]);
        assert_eq!(events(&[0xE2, 0x1B, b'[', b'A']), [bad, Event::Csi(b'A')]);
        assert_eq!(events(&[0xC3, 0xC3, 0xA9]), [bad, Event::Print('é')]);
    }

    #[test]
    fn overlong_surrogate_and_out_of_range_are_replaced() {
        let bad = Event::Print(REPLACEMENT);
        assert_eq!(events(&[0xE0, 0x80, 0x80]), [bad]);
        assert_eq!(events(&[0xED, 0xA0, 0x80]), [bad]);
        assert_eq!(events(&[0xF4, 0x90, 0x80, 0x80]), [bad]);
    }

    #[test]
    fn c1_controls_in_utf8_are_ignored() {
        assert_eq!(events(&[0xC2, 0x9B, b'A']), [Event::Print('A')]);
        assert_eq!(events(&[0xC2, 0x80]), []);
    }

    #[test]
    fn non_ascii_inside_a_sequence_is_ignored_without_ending_it() {
        let mut parser = Parser::new();
        assert_eq!(run(&mut parser, "\x1b[1é2A".as_bytes()), [Event::Csi(b'A')]);
        assert_eq!(parser.param(0), 12);
    }
}
