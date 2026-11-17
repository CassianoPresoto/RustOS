//! Teclado: transforma `KeyEvent` (tecla física, HID) na `Key` do terminal,
//! com o layout, os modificadores e as teclas mortas. Não conhece o dispositivo.

mod layout;

use rustos_hal::{KeyCode, KeyEvent};

use crate::Key;
use layout::{ABNT2, NONE, Row, US, accent, compose, find, is_dead};

const SHIFT_LEFT: u8 = 1 << 0;
const SHIFT_RIGHT: u8 = 1 << 1;
const CTRL_LEFT: u8 = 1 << 2;
const CTRL_RIGHT: u8 = 1 << 3;
const ALT_LEFT: u8 = 1 << 4;
const ALT_RIGHT: u8 = 1 << 5;
const CAPS_ON: u8 = 1 << 6;
const CAPS_HELD: u8 = 1 << 7;

const QUEUE: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// `us` do xkeyboard-config.
    Us,
    /// `br(abnt2)`: AltGr é o Alt direito.
    BrAbnt2,
}

/// Estado do teclado: modificadores, Caps Lock e a tecla morta pendente.
#[derive(Clone, Copy, Debug)]
pub struct Keyboard {
    layout: Layout,
    mods: u8,
    dead: Option<char>,
    queue: [Option<Key>; QUEUE],
    queued: usize,
}

impl Keyboard {
    pub const fn new(layout: Layout) -> Self {
        Self {
            layout,
            mods: 0,
            dead: None,
            queue: [None; QUEUE],
            queued: 0,
        }
    }

    /// Troca o layout e descarta a tecla morta pendente.
    pub fn set_layout(&mut self, layout: Layout) {
        self.layout = layout;
        self.dead = None;
    }

    /// Atualiza modificadores; devolve a tecla de terminal quando há uma.
    /// As teclas que sobram do mesmo evento saem por [`Self::next_queued`].
    pub fn feed(&mut self, event: KeyEvent) -> Option<Key> {
        self.process(event);
        self.next_queued()
    }

    /// Próxima tecla que um `feed` anterior deixou na fila.
    pub fn next_queued(&mut self) -> Option<Key> {
        if self.queued == 0 {
            return None;
        }
        let key = self.queue[0];
        self.queue.copy_within(1.., 0);
        self.queued -= 1;
        self.queue[self.queued] = None;
        key
    }

    fn push(&mut self, key: Key) {
        if self.queued < QUEUE {
            self.queue[self.queued] = Some(key);
            self.queued += 1;
        }
    }

    fn shift(&self) -> bool {
        self.mods & (SHIFT_LEFT | SHIFT_RIGHT) != 0
    }

    fn ctrl(&self) -> bool {
        self.mods & (CTRL_LEFT | CTRL_RIGHT) != 0
    }

    fn altgr(&self) -> bool {
        self.layout == Layout::BrAbnt2 && self.mods & ALT_RIGHT != 0
    }

    fn alt(&self) -> bool {
        self.mods & ALT_LEFT != 0 || (self.layout == Layout::Us && self.mods & ALT_RIGHT != 0)
    }

    fn set(&mut self, bit: u8, on: bool) {
        if on {
            self.mods |= bit;
        } else {
            self.mods &= !bit;
        }
    }

    fn process(&mut self, event: KeyEvent) {
        let pressed = event.pressed;
        match event.code {
            KeyCode::LEFT_SHIFT => self.set(SHIFT_LEFT, pressed),
            KeyCode::RIGHT_SHIFT => self.set(SHIFT_RIGHT, pressed),
            KeyCode::LEFT_CTRL => self.set(CTRL_LEFT, pressed),
            KeyCode::RIGHT_CTRL => self.set(CTRL_RIGHT, pressed),
            KeyCode::LEFT_ALT => self.set(ALT_LEFT, pressed),
            KeyCode::RIGHT_ALT => self.set(ALT_RIGHT, pressed),
            KeyCode::CAPS_LOCK => {
                if pressed && self.mods & CAPS_HELD == 0 {
                    self.mods ^= CAPS_ON;
                }
                self.set(CAPS_HELD, pressed);
            }
            code if pressed => self.press(code),
            _ => {}
        }
    }

    fn press(&mut self, code: KeyCode) {
        if let Some(key) = self.special(code) {
            self.flush_dead();
            self.push(key);
        } else if let Some(c) = self.symbol(code) {
            self.press_char(c);
        }
    }

    fn special(&self, code: KeyCode) -> Option<Key> {
        let f_base = KeyCode::F1.0;
        Some(match code {
            KeyCode::ENTER | KeyCode::KP_ENTER => Key::Enter,
            KeyCode::ESCAPE => Key::Escape,
            KeyCode::BACKSPACE => Key::Backspace,
            KeyCode::TAB if self.shift() => Key::BackTab,
            KeyCode::TAB => Key::Tab,
            KeyCode::UP => Key::Up,
            KeyCode::DOWN => Key::Down,
            KeyCode::LEFT => Key::Left,
            KeyCode::RIGHT => Key::Right,
            KeyCode::HOME => Key::Home,
            KeyCode::END => Key::End,
            KeyCode::PAGE_UP => Key::PageUp,
            KeyCode::PAGE_DOWN => Key::PageDown,
            KeyCode::INSERT => Key::Insert,
            KeyCode::DELETE => Key::Delete,
            KeyCode(n) if (KeyCode::F1.0..=KeyCode::F12.0).contains(&n) => Key::F(n - f_base + 1),
            _ => return None,
        })
    }

    fn table(&self) -> &'static [Row] {
        match self.layout {
            Layout::Us => &US,
            Layout::BrAbnt2 => &ABNT2,
        }
    }

    /// O caractere da tecla com os modificadores de agora; `None` se não há.
    fn symbol(&self, code: KeyCode) -> Option<char> {
        if let Some(c) = self.keypad(code) {
            return Some(c);
        }
        let row = find(self.table(), code.0)?;
        let shift = usize::from(self.shift());
        let c = if self.altgr() && row[2 + shift] != NONE {
            row[2 + shift]
        } else {
            let caps = self.mods & CAPS_ON != 0 && is_letter(row[0]);
            row[usize::from(self.shift() != caps)]
        };
        (c != NONE).then_some(c)
    }

    /// Teclado numérico sempre com Num Lock ligado (DEBT-0010).
    fn keypad(&self, code: KeyCode) -> Option<char> {
        let abnt2 = self.layout == Layout::BrAbnt2;
        Some(match code {
            KeyCode::KP_DIVIDE => '/',
            KeyCode::KP_MULTIPLY => '*',
            KeyCode::KP_MINUS => '-',
            KeyCode::KP_PLUS => '+',
            KeyCode(n) if (KeyCode::KP_1.0..KeyCode::KP_0.0).contains(&n) => {
                char::from(b'1' + (n - KeyCode::KP_1.0))
            }
            KeyCode::KP_0 => '0',
            KeyCode::KP_PERIOD if abnt2 => ',',
            KeyCode::KP_PERIOD => '.',
            KeyCode::KP_COMMA if abnt2 => '.',
            KeyCode::KP_COMMA => ',',
            _ => return None,
        })
    }

    fn press_char(&mut self, c: char) {
        if self.ctrl() {
            self.flush_dead();
            if c.is_ascii() {
                self.push(Key::Ctrl(c.to_ascii_lowercase()));
            }
        } else if self.alt() && !is_dead(c) {
            self.flush_dead();
            self.push(Key::Escape);
            self.push(Key::Char(c));
        } else if is_dead(c) {
            self.press_dead(c);
        } else {
            self.press_plain(c);
        }
    }

    fn press_dead(&mut self, c: char) {
        match self.dead.replace(c) {
            Some(previous) if previous == c => {
                self.dead = None;
                self.push(Key::Char(accent(c)));
            }
            Some(previous) => self.push(Key::Char(accent(previous))),
            None => {}
        }
    }

    fn press_plain(&mut self, c: char) {
        let Some(dead) = self.dead.take() else {
            self.push(Key::Char(c));
            return;
        };
        match compose(dead, c) {
            Some(composed) => self.push(Key::Char(composed)),
            None if c == ' ' => self.push(Key::Char(accent(dead))),
            None => {
                self.push(Key::Char(accent(dead)));
                self.push(Key::Char(c));
            }
        }
    }

    fn flush_dead(&mut self) {
        if let Some(dead) = self.dead.take() {
            self.push(Key::Char(accent(dead)));
        }
    }
}

fn is_letter(c: char) -> bool {
    c.is_ascii_lowercase() || c == 'ç'
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIFT: KeyCode = KeyCode::LEFT_SHIFT;
    const ALTGR: KeyCode = KeyCode::RIGHT_ALT;
    const ACUTE_KEY: u8 = 0x2F;
    const TILDE_KEY: u8 = 0x34;
    const DIGIT_6: u8 = 0x23;

    fn event(code: KeyCode, pressed: bool) -> KeyEvent {
        KeyEvent { code, pressed }
    }

    fn drain(kbd: &mut Keyboard, first: Option<Key>) -> Vec<Key> {
        let mut out: Vec<Key> = first.into_iter().collect();
        while let Some(key) = kbd.next_queued() {
            out.push(key);
        }
        out
    }

    fn tap(kbd: &mut Keyboard, code: u8) -> Vec<Key> {
        let down = kbd.feed(event(KeyCode(code), true));
        let mut out = drain(kbd, down);
        let up = kbd.feed(event(KeyCode(code), false));
        out.extend(drain(kbd, up));
        out
    }

    fn chord(kbd: &mut Keyboard, held: &[KeyCode], code: u8) -> Vec<Key> {
        for &m in held {
            assert_eq!(kbd.feed(event(m, true)), None);
        }
        let out = tap(kbd, code);
        for &m in held {
            assert_eq!(kbd.feed(event(m, false)), None);
        }
        out
    }

    fn letter(c: char) -> u8 {
        0x04 + (c as u8 - b'a')
    }

    fn chars(keys: &[Key]) -> String {
        keys.iter()
            .map(|k| match k {
                Key::Char(c) => *c,
                other => panic!("esperava caractere, veio {other:?}"),
            })
            .collect()
    }

    fn typed(layout: Layout, held: &[KeyCode], code: u8) -> String {
        chars(&chord(&mut Keyboard::new(layout), held, code))
    }

    fn us(code: u8) -> String {
        typed(Layout::Us, &[], code)
    }

    fn us_shift(code: u8) -> String {
        typed(Layout::Us, &[SHIFT], code)
    }

    fn br(code: u8) -> String {
        typed(Layout::BrAbnt2, &[], code)
    }

    fn br_shift(code: u8) -> String {
        typed(Layout::BrAbnt2, &[SHIFT], code)
    }

    fn br_altgr(code: u8) -> String {
        typed(Layout::BrAbnt2, &[ALTGR], code)
    }

    fn br_altgr_shift(code: u8) -> String {
        typed(Layout::BrAbnt2, &[ALTGR, SHIFT], code)
    }

    fn row(keys: &[u8], f: fn(u8) -> String) -> String {
        keys.iter().map(|&k| f(k)).collect()
    }

    const LETTERS: core::ops::RangeInclusive<u8> = 0x04..=0x1D;
    const DIGITS: core::ops::RangeInclusive<u8> = 0x1E..=0x27;

    #[test]
    fn us_letters_are_lowercase_and_shift_makes_them_uppercase() {
        let lower: String = LETTERS.map(us).collect();
        let upper: String = LETTERS.map(us_shift).collect();
        assert_eq!(lower, "abcdefghijklmnopqrstuvwxyz");
        assert_eq!(upper, "ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    }

    #[test]
    fn us_digits_and_their_shifted_symbols() {
        let plain: String = DIGITS.map(us).collect();
        let shifted: String = DIGITS.map(us_shift).collect();
        assert_eq!(plain, "1234567890");
        assert_eq!(shifted, "!@#$%^&*()");
    }

    #[test]
    fn us_punctuation_keys_have_two_levels() {
        let keys = [
            0x2C, 0x2D, 0x2E, 0x2F, 0x30, 0x31, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38,
        ];
        assert_eq!(row(&keys, us), " -=[]\\;'`,./");
        assert_eq!(row(&keys, us_shift), " _+{}|:\"~<>?");
    }

    #[test]
    fn us_has_no_dead_keys() {
        assert_eq!(us(0x34), "'");
        assert_eq!(us(0x35), "`");
        assert_eq!(us_shift(0x35), "~");
    }

    #[test]
    fn the_editing_keys_become_terminal_keys() {
        let mut kbd = Keyboard::new(Layout::Us);
        assert_eq!(tap(&mut kbd, 0x28), [Key::Enter]);
        assert_eq!(tap(&mut kbd, 0x58), [Key::Enter]);
        assert_eq!(tap(&mut kbd, 0x29), [Key::Escape]);
        assert_eq!(tap(&mut kbd, 0x2A), [Key::Backspace]);
        assert_eq!(tap(&mut kbd, 0x2B), [Key::Tab]);
        assert_eq!(chord(&mut kbd, &[SHIFT], 0x2B), [Key::BackTab]);
    }

    #[test]
    fn navigation_keys_become_terminal_keys() {
        let mut kbd = Keyboard::new(Layout::Us);
        let expected = [
            (0x52, Key::Up),
            (0x51, Key::Down),
            (0x50, Key::Left),
            (0x4F, Key::Right),
            (0x4A, Key::Home),
            (0x4D, Key::End),
            (0x4B, Key::PageUp),
            (0x4E, Key::PageDown),
            (0x49, Key::Insert),
            (0x4C, Key::Delete),
        ];
        for (code, key) in expected {
            assert_eq!(tap(&mut kbd, code), [key]);
        }
    }

    #[test]
    fn function_keys_f1_to_f12_carry_their_number() {
        for layout in [Layout::Us, Layout::BrAbnt2] {
            let mut kbd = Keyboard::new(layout);
            for n in 1..=12u8 {
                assert_eq!(tap(&mut kbd, 0x39 + n), [Key::F(n)]);
            }
        }
    }

    #[test]
    fn keys_without_a_terminal_meaning_produce_nothing() {
        let mut kbd = Keyboard::new(Layout::Us);
        for code in [0x46, 0x47, 0x48, 0x53, 0x65, 0x00, 0xFF] {
            assert_eq!(tap(&mut kbd, code), []);
        }
    }

    #[test]
    fn the_keypad_types_digits_and_operators() {
        let mut kbd = Keyboard::new(Layout::Us);
        let keys: Vec<Key> = (0x59..=0x62).flat_map(|c| tap(&mut kbd, c)).collect();
        assert_eq!(chars(&keys), "1234567890");
        let ops: Vec<Key> = [0x54, 0x55, 0x56, 0x57, 0x63]
            .into_iter()
            .flat_map(|c| tap(&mut kbd, c))
            .collect();
        assert_eq!(chars(&ops), "/*-+.");
    }

    #[test]
    fn the_abnt2_keypad_period_key_types_a_comma() {
        assert_eq!(br(0x63), ",");
        assert_eq!(br(0x85), ".");
        assert_eq!(us(0x63), ".");
    }

    #[test]
    fn ctrl_with_a_letter_becomes_ctrl_of_the_lowercase_letter() {
        let mut kbd = Keyboard::new(Layout::Us);
        assert_eq!(
            chord(&mut kbd, &[KeyCode::LEFT_CTRL], letter('c')),
            [Key::Ctrl('c')]
        );
        assert_eq!(
            chord(&mut kbd, &[KeyCode::RIGHT_CTRL, SHIFT], letter('z')),
            [Key::Ctrl('z')]
        );
        for c in 'a'..='z' {
            assert_eq!(
                chord(&mut kbd, &[KeyCode::LEFT_CTRL], letter(c)),
                [Key::Ctrl(c)]
            );
        }
    }

    #[test]
    fn ctrl_with_symbols_follows_the_terminal_table() {
        let mut kbd = Keyboard::new(Layout::Us);
        let ctrl = [KeyCode::LEFT_CTRL];
        assert_eq!(chord(&mut kbd, &ctrl, 0x2C), [Key::Ctrl(' ')]);
        assert_eq!(chord(&mut kbd, &ctrl, 0x2F), [Key::Ctrl('[')]);
        assert_eq!(chord(&mut kbd, &ctrl, 0x31), [Key::Ctrl('\\')]);
    }

    #[test]
    fn ctrl_with_a_non_ascii_character_produces_nothing() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        assert_eq!(chord(&mut kbd, &[KeyCode::LEFT_CTRL], 0x33), []);
    }

    #[test]
    fn releasing_ctrl_goes_back_to_plain_characters() {
        let mut kbd = Keyboard::new(Layout::Us);
        assert_eq!(kbd.feed(event(KeyCode::LEFT_CTRL, true)), None);
        assert_eq!(tap(&mut kbd, letter('c')), [Key::Ctrl('c')]);
        assert_eq!(kbd.feed(event(KeyCode::LEFT_CTRL, false)), None);
        assert_eq!(tap(&mut kbd, letter('c')), [Key::Char('c')]);
    }

    #[test]
    fn releasing_shift_goes_back_to_lowercase() {
        let mut kbd = Keyboard::new(Layout::Us);
        assert_eq!(kbd.feed(event(SHIFT, true)), None);
        assert_eq!(tap(&mut kbd, letter('a')), [Key::Char('A')]);
        assert_eq!(kbd.feed(event(SHIFT, false)), None);
        assert_eq!(tap(&mut kbd, letter('a')), [Key::Char('a')]);
    }

    #[test]
    fn shift_stays_active_until_both_shift_keys_are_released() {
        let mut kbd = Keyboard::new(Layout::Us);
        kbd.feed(event(KeyCode::LEFT_SHIFT, true));
        kbd.feed(event(KeyCode::RIGHT_SHIFT, true));
        kbd.feed(event(KeyCode::LEFT_SHIFT, false));
        assert_eq!(tap(&mut kbd, letter('a')), [Key::Char('A')]);
        kbd.feed(event(KeyCode::RIGHT_SHIFT, false));
        assert_eq!(tap(&mut kbd, letter('a')), [Key::Char('a')]);
    }

    #[test]
    fn caps_lock_changes_only_letters() {
        let mut kbd = Keyboard::new(Layout::Us);
        assert_eq!(tap(&mut kbd, 0x39), []);
        assert_eq!(tap(&mut kbd, letter('a')), [Key::Char('A')]);
        assert_eq!(tap(&mut kbd, 0x1E), [Key::Char('1')]);
        assert_eq!(tap(&mut kbd, 0x2D), [Key::Char('-')]);
        assert_eq!(tap(&mut kbd, 0x2C), [Key::Char(' ')]);
        assert_eq!(chord(&mut kbd, &[SHIFT], 0x1E), [Key::Char('!')]);
    }

    #[test]
    fn shift_and_caps_together_give_lowercase_letters() {
        let mut kbd = Keyboard::new(Layout::Us);
        tap(&mut kbd, 0x39);
        assert_eq!(chord(&mut kbd, &[SHIFT], letter('a')), [Key::Char('a')]);
        tap(&mut kbd, 0x39);
        assert_eq!(chord(&mut kbd, &[SHIFT], letter('a')), [Key::Char('A')]);
    }

    #[test]
    fn caps_lock_toggles_once_per_physical_press() {
        let mut kbd = Keyboard::new(Layout::Us);
        kbd.feed(event(KeyCode::CAPS_LOCK, true));
        kbd.feed(event(KeyCode::CAPS_LOCK, true));
        kbd.feed(event(KeyCode::CAPS_LOCK, false));
        assert_eq!(tap(&mut kbd, letter('a')), [Key::Char('A')]);
        tap(&mut kbd, 0x39);
        assert_eq!(tap(&mut kbd, letter('a')), [Key::Char('a')]);
    }

    #[test]
    fn alt_with_a_character_sends_escape_first() {
        let mut kbd = Keyboard::new(Layout::Us);
        assert_eq!(
            chord(&mut kbd, &[KeyCode::LEFT_ALT], letter('f')),
            [Key::Escape, Key::Char('f')]
        );
        assert_eq!(
            chord(&mut kbd, &[KeyCode::RIGHT_ALT], letter('b')),
            [Key::Escape, Key::Char('b')]
        );
    }

    #[test]
    fn feed_alone_delivers_every_key_of_a_multi_key_press() {
        let mut kbd = Keyboard::new(Layout::Us);
        kbd.feed(event(KeyCode::LEFT_ALT, true));
        let mut out = vec![kbd.feed(event(KeyCode(letter('f')), true))];
        out.push(kbd.feed(event(KeyCode(letter('f')), false)));
        out.push(kbd.feed(event(KeyCode::LEFT_ALT, false)));
        assert_eq!(out, [Some(Key::Escape), Some(Key::Char('f')), None]);
    }

    #[test]
    fn abnt2_letters_match_the_us_positions() {
        let lower: String = LETTERS.map(br).collect();
        let upper: String = LETTERS.map(br_shift).collect();
        assert_eq!(lower, "abcdefghijklmnopqrstuvwxyz");
        assert_eq!(upper, "ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    }

    #[test]
    fn abnt2_digits_and_their_shifted_symbols() {
        let plain: String = DIGITS.map(br).collect();
        assert_eq!(plain, "1234567890");
        let shifted: String = DIGITS.filter(|&c| c != DIGIT_6).map(br_shift).collect();
        assert_eq!(shifted, "!@#$%&*()");
    }

    #[test]
    fn abnt2_puts_c_cedilla_where_the_us_has_the_semicolon() {
        assert_eq!(br(0x33), "ç");
        assert_eq!(br_shift(0x33), "Ç");
        assert_eq!(us(0x33), ";");
    }

    #[test]
    fn abnt2_moves_the_semicolon_and_colon_to_the_slash_position() {
        assert_eq!(br(0x38), ";");
        assert_eq!(br_shift(0x38), ":");
    }

    #[test]
    fn abnt2_has_the_extra_slash_and_question_key() {
        assert_eq!(br(0x87), "/");
        assert_eq!(br_shift(0x87), "?");
    }

    #[test]
    fn abnt2_punctuation_and_bracket_keys() {
        let keys = [0x2C, 0x2D, 0x2E, 0x30, 0x31, 0x32, 0x35, 0x36, 0x37, 0x64];
        assert_eq!(row(&keys, br), " -=[]]',.\\");
        assert_eq!(row(&keys, br_shift), " _+{}}\"<>|");
    }

    #[test]
    fn abnt2_caps_lock_capitalizes_c_cedilla() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        tap(&mut kbd, 0x39);
        assert_eq!(tap(&mut kbd, 0x33), [Key::Char('Ç')]);
        assert_eq!(chord(&mut kbd, &[SHIFT], 0x33), [Key::Char('ç')]);
        assert_eq!(tap(&mut kbd, 0x38), [Key::Char(';')]);
    }

    #[test]
    fn altgr_types_slash_question_and_degree_on_q_w_e() {
        assert_eq!(br_altgr(letter('q')), "/");
        assert_eq!(br_altgr(letter('w')), "?");
        assert_eq!(br_altgr(letter('e')), "°");
    }

    #[test]
    fn altgr_types_the_ordinal_indicators_and_the_symbol_row() {
        assert_eq!(br_altgr(0x30), "ª");
        assert_eq!(br_altgr(0x31), "º");
        assert_eq!(
            row(&[0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23], br_altgr),
            "¹²³£¢¬"
        );
        assert_eq!(br_altgr(0x2E), "§");
        assert_eq!(row(&[0x24, 0x25, 0x26, 0x27, 0x2D], br_altgr), "{[]}\\");
    }

    #[test]
    fn altgr_with_shift_reaches_the_fourth_level() {
        assert_eq!(br_altgr_shift(0x87), "¿");
        assert_eq!(br_altgr_shift(0x1E), "¡");
        assert_eq!(br_altgr_shift(0x1F), "½");
    }

    #[test]
    fn altgr_on_a_key_without_a_third_level_types_the_base_character() {
        assert_eq!(br_altgr(letter('h')), "h");
        assert_eq!(br_altgr_shift(letter('h')), "H");
    }

    #[test]
    fn altgr_is_just_alt_on_the_us_layout() {
        let mut kbd = Keyboard::new(Layout::Us);
        assert_eq!(
            chord(&mut kbd, &[ALTGR], letter('q')),
            [Key::Escape, Key::Char('q')]
        );
    }

    #[test]
    fn releasing_altgr_goes_back_to_the_base_level() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        kbd.feed(event(ALTGR, true));
        assert_eq!(tap(&mut kbd, letter('q')), [Key::Char('/')]);
        kbd.feed(event(ALTGR, false));
        assert_eq!(tap(&mut kbd, letter('q')), [Key::Char('q')]);
    }

    #[test]
    fn altgr_does_not_send_escape_on_the_abnt2_layout() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        assert_eq!(chord(&mut kbd, &[ALTGR], letter('q')), [Key::Char('/')]);
    }

    #[test]
    fn set_layout_switches_the_map_in_place() {
        let mut kbd = Keyboard::new(Layout::Us);
        assert_eq!(tap(&mut kbd, 0x33), [Key::Char(';')]);
        kbd.set_layout(Layout::BrAbnt2);
        assert_eq!(tap(&mut kbd, 0x33), [Key::Char('ç')]);
    }

    fn dead(kbd: &mut Keyboard, accent: char) -> Vec<Key> {
        match accent {
            '´' => chord(kbd, &[], ACUTE_KEY),
            '`' => chord(kbd, &[SHIFT], ACUTE_KEY),
            '~' => chord(kbd, &[], TILDE_KEY),
            '^' => chord(kbd, &[SHIFT], TILDE_KEY),
            '¨' => chord(kbd, &[SHIFT], DIGIT_6),
            _ => unreachable!(),
        }
    }

    fn compose_text(accent: char, base: char) -> String {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        assert_eq!(dead(&mut kbd, accent), []);
        let held: &[KeyCode] = if base.is_ascii_uppercase() {
            &[SHIFT]
        } else {
            &[]
        };
        let code = letter(base.to_ascii_lowercase());
        chars(&chord(&mut kbd, held, code))
    }

    #[test]
    fn every_dead_key_combination_with_vowels_and_n() {
        let table = [
            ('´', "aeiouy", "áéíóúý"),
            ('`', "aeiou", "àèìòù"),
            ('~', "aon", "ãõñ"),
            ('^', "aeiou", "âêîôû"),
            ('¨', "aeiouy", "äëïöüÿ"),
        ];
        for (accent, bases, expected) in table {
            for (base, want) in bases.chars().zip(expected.chars()) {
                assert_eq!(
                    compose_text(accent, base),
                    want.to_string(),
                    "{accent}{base}"
                );
                let upper = base.to_ascii_uppercase();
                let want_upper: String = want.to_uppercase().collect();
                if upper != 'Y' || accent == '´' {
                    assert_eq!(compose_text(accent, upper), want_upper, "{accent}{upper}");
                }
            }
        }
    }

    #[test]
    fn acute_c_gives_c_cedilla_as_on_the_brazilian_layout() {
        assert_eq!(compose_text('´', 'c'), "ç");
        assert_eq!(compose_text('´', 'C'), "Ç");
    }

    #[test]
    fn a_dead_key_followed_by_space_types_the_accent_itself() {
        for accent in ['´', '`', '~', '^', '¨'] {
            let mut kbd = Keyboard::new(Layout::BrAbnt2);
            assert_eq!(dead(&mut kbd, accent), []);
            assert_eq!(chars(&tap(&mut kbd, 0x2C)), accent.to_string());
            assert_eq!(chars(&tap(&mut kbd, 0x2C)), " ");
        }
    }

    #[test]
    fn a_dead_key_followed_by_a_letter_that_does_not_combine_types_both() {
        for accent in ['´', '`', '~', '^', '¨'] {
            let mut kbd = Keyboard::new(Layout::BrAbnt2);
            dead(&mut kbd, accent);
            assert_eq!(
                tap(&mut kbd, letter('x')),
                [Key::Char(accent), Key::Char('x')]
            );
            assert_eq!(tap(&mut kbd, letter('x')), [Key::Char('x')]);
        }
    }

    #[test]
    fn a_dead_key_followed_by_a_digit_types_both() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        dead(&mut kbd, '~');
        assert_eq!(tap(&mut kbd, 0x1E), [Key::Char('~'), Key::Char('1')]);
    }

    #[test]
    fn the_extra_key_of_a_non_combining_letter_is_delivered_by_feed_alone() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        dead(&mut kbd, '´');
        assert_eq!(
            kbd.feed(event(KeyCode(letter('x')), true)),
            Some(Key::Char('´'))
        );
        assert_eq!(
            kbd.feed(event(KeyCode(letter('x')), false)),
            Some(Key::Char('x'))
        );
        assert_eq!(
            kbd.feed(event(KeyCode(letter('x')), true)),
            Some(Key::Char('x'))
        );
    }

    #[test]
    fn pressing_the_same_dead_key_twice_types_the_accent_once() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        dead(&mut kbd, '´');
        assert_eq!(dead(&mut kbd, '´'), [Key::Char('´')]);
        assert_eq!(tap(&mut kbd, letter('a')), [Key::Char('a')]);
    }

    #[test]
    fn two_different_dead_keys_type_the_first_and_wait_with_the_second() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        dead(&mut kbd, '´');
        assert_eq!(dead(&mut kbd, '^'), [Key::Char('´')]);
        assert_eq!(tap(&mut kbd, letter('e')), [Key::Char('ê')]);
    }

    #[test]
    fn a_pending_dead_key_survives_modifier_presses_and_releases() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        dead(&mut kbd, '´');
        assert_eq!(kbd.feed(event(SHIFT, true)), None);
        assert_eq!(tap(&mut kbd, letter('a')), [Key::Char('Á')]);
        assert_eq!(kbd.feed(event(SHIFT, false)), None);
    }

    #[test]
    fn a_dead_key_followed_by_a_terminal_key_types_the_accent_first() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        dead(&mut kbd, '^');
        assert_eq!(tap(&mut kbd, 0x28), [Key::Char('^'), Key::Enter]);
        dead(&mut kbd, '^');
        assert_eq!(tap(&mut kbd, 0x50), [Key::Char('^'), Key::Left]);
    }

    #[test]
    fn a_dead_key_followed_by_ctrl_letter_types_the_accent_then_the_control() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        dead(&mut kbd, '~');
        assert_eq!(
            chord(&mut kbd, &[KeyCode::LEFT_CTRL], letter('c')),
            [Key::Char('~'), Key::Ctrl('c')]
        );
    }

    #[test]
    fn set_layout_drops_the_pending_dead_key() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        dead(&mut kbd, '´');
        kbd.set_layout(Layout::Us);
        assert_eq!(tap(&mut kbd, letter('a')), [Key::Char('a')]);
    }

    #[test]
    fn altgr_c_cedilla_key_is_the_acute_dead_key() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        assert_eq!(chord(&mut kbd, &[ALTGR], 0x33), []);
        assert_eq!(tap(&mut kbd, letter('e')), [Key::Char('é')]);
    }

    #[test]
    fn the_brazilian_words_come_out_right() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        let mut out = Vec::new();
        out.extend(tap(&mut kbd, letter('a')));
        out.extend(tap(&mut kbd, 0x33));
        out.extend(chord(&mut kbd, &[], TILDE_KEY));
        out.extend(tap(&mut kbd, letter('a')));
        out.extend(tap(&mut kbd, letter('o')));
        out.extend(tap(&mut kbd, 0x2C));
        out.extend(chord(&mut kbd, &[], ACUTE_KEY));
        out.extend(tap(&mut kbd, letter('a')));
        out.extend(dead(&mut kbd, '^'));
        out.extend(tap(&mut kbd, letter('e')));
        out.extend(dead(&mut kbd, '¨'));
        out.extend(tap(&mut kbd, letter('u')));
        assert_eq!(chars(&out), "ação áêü");
    }

    #[test]
    fn every_table_is_sorted_by_hid_code_without_duplicates() {
        for table in [&US[..], &ABNT2[..]] {
            assert!(table.windows(2).all(|w| w[0].0 < w[1].0));
        }
    }

    #[test]
    fn dead_keys_never_leak_out_as_characters() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        for held in [&[][..], &[SHIFT], &[ALTGR], &[ALTGR, SHIFT]] {
            for code in 0..=0xFF {
                for key in chord(&mut kbd, held, code) {
                    if let Key::Char(c) = key {
                        assert!(!is_dead(c), "{code:#04x}");
                    }
                }
                kbd.set_layout(Layout::BrAbnt2);
            }
        }
    }

    #[test]
    fn the_queue_never_overflows_when_feed_is_the_only_consumer() {
        let mut kbd = Keyboard::new(Layout::BrAbnt2);
        kbd.feed(event(KeyCode::LEFT_ALT, true));
        for _ in 0..50 {
            kbd.feed(event(KeyCode(letter('x')), true));
        }
        assert!(kbd.queued <= QUEUE);
    }
}
