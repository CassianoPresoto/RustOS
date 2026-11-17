use super::*;

macro_rules! term {
    ($t:ident, $cols:expr, $rows:expr) => {
        let mut storage = vec![Cell::BLANK; Terminal::cells_needed($cols, $rows)];
        #[allow(unused_mut, reason = "nem todo teste escreve no terminal")]
        let mut $t = Terminal::new(&mut storage, $cols, $rows).unwrap();
    };
}

macro_rules! full {
    ($t:ident, $cols:expr, $rows:expr, $lines:expr) => {
        let mut main = vec![Cell::BLANK; Terminal::cells_needed($cols, $rows)];
        let mut alt = vec![Cell::BLANK; Terminal::cells_needed($cols, $rows)];
        let mut hist = vec![Cell::BLANK; Terminal::history_cells_needed($cols, $lines)];
        #[allow(unused_mut, reason = "nem todo teste escreve no terminal")]
        let mut $t = Terminal::new(&mut main, $cols, $rows)
            .unwrap()
            .with_alternate(&mut alt)
            .unwrap()
            .with_history(&mut hist);
    };
}

fn line(t: &Terminal<'_>, row: usize) -> String {
    let text: String = t.row(row).iter().map(|c| c.ch).collect();
    text.trim_end().to_string()
}

fn screen(t: &Terminal<'_>) -> Vec<String> {
    (0..t.rows()).map(|row| line(t, row)).collect()
}

fn at(t: &Terminal<'_>, row: usize, col: usize) -> Cell {
    t.row(row)[col]
}

fn pos(t: &Terminal<'_>) -> (usize, usize) {
    (t.cursor().row, t.cursor().col)
}

fn dirty(t: &mut Terminal<'_>) -> Vec<usize> {
    t.take_dirty().collect()
}

fn reply(t: &mut Terminal<'_>) -> Vec<u8> {
    let mut out = [0u8; 128];
    let n = t.take_reply(&mut out);
    out[..n].to_vec()
}

fn numbered(t: &mut Terminal<'_>, count: usize) {
    for i in 1..=count {
        if i > 1 {
            t.write(b"\r\n");
        }
        t.write(i.to_string().as_bytes());
    }
}

fn snapshot(t: &Terminal<'_>) -> (Vec<Cell>, Cursor) {
    let cells = (0..t.rows()).flat_map(|r| t.row(r).to_vec()).collect();
    (cells, t.cursor())
}

mod screens;

// ---- construção ----

#[test]
fn new_rejects_invalid_geometry() {
    let mut cells = vec![Cell::BLANK; 100];
    assert!(Terminal::new(&mut cells, 0, 5).is_none());
    assert!(Terminal::new(&mut cells, 5, 0).is_none());
    assert!(Terminal::new(&mut cells, 11, 10).is_none());
    assert!(Terminal::new(&mut cells, usize::MAX, 2).is_none());
    assert!(Terminal::new(&mut cells[..49], 10, 5).is_none());
}

#[test]
fn new_accepts_the_exact_slice_and_blanks_it() {
    let mut cells = vec![
        Cell {
            ch: 'x',
            ..Cell::BLANK
        };
        50
    ];
    let t = Terminal::new(&mut cells, 10, 5).unwrap();
    assert_eq!((t.cols(), t.rows()), (10, 5));
    assert!(screen(&t).iter().all(String::is_empty));
    assert_eq!(pos(&t), (0, 0));
    assert!(t.cursor().visible);
}

#[test]
fn new_enforces_the_row_limit() {
    let mut cells = vec![Cell::BLANK; 257 * 2];
    assert!(Terminal::new(&mut cells, 2, MAX_ROWS).is_some());
    assert!(Terminal::new(&mut cells, 2, MAX_ROWS + 1).is_none());
}

#[test]
fn cells_needed_multiplies_and_saturates() {
    assert_eq!(Terminal::cells_needed(80, 25), 2000);
    assert_eq!(Terminal::cells_needed(0, 25), 0);
    assert_eq!(Terminal::cells_needed(usize::MAX, 2), usize::MAX);
}

#[test]
fn row_outside_the_grid_is_empty() {
    term!(t, 4, 2);
    assert_eq!(t.row(1).len(), 4);
    assert_eq!(t.row(2).len(), 0);
    assert_eq!(t.row(usize::MAX).len(), 0);
}

#[test]
fn writing_text_advances_the_cursor() {
    term!(t, 10, 3);
    t.write(b"hello");
    assert_eq!(line(&t, 0), "hello");
    assert_eq!(pos(&t), (0, 5));
    t.write(b"");
    assert_eq!(pos(&t), (0, 5));
}

#[test]
fn printed_cells_carry_the_current_pen() {
    term!(t, 10, 2);
    t.write(b"\x1b[1;31;44mab");
    for col in 0..2 {
        let cell = at(&t, 0, col);
        assert_eq!((cell.fg, cell.bg), (1, 4));
        assert!(cell.flags.contains(Flags::BOLD));
        assert!(!cell.flags.contains(Flags::DEFAULT_FG));
        assert!(!cell.flags.contains(Flags::DEFAULT_BG));
    }
    assert_eq!(at(&t, 0, 2), Cell::BLANK);
}

// ---- quebra de linha ----

#[test]
fn last_column_write_defers_the_wrap() {
    term!(t, 5, 3);
    t.write(b"abcde");
    assert_eq!(pos(&t), (0, 4));
    assert_eq!(screen(&t), ["abcde", "", ""]);
    t.write(b"f");
    assert_eq!(pos(&t), (1, 1));
    assert_eq!(screen(&t), ["abcde", "f", ""]);
}

#[test]
fn carriage_return_after_pending_wrap_leaves_no_blank_line() {
    term!(t, 5, 3);
    t.write(b"abcde\r\nX");
    assert_eq!(screen(&t), ["abcde", "X", ""]);
}

#[test]
fn line_feed_clears_the_pending_wrap_and_keeps_the_column() {
    term!(t, 5, 3);
    t.write(b"abcde\nX");
    assert_eq!(screen(&t), ["abcde", "    X", ""]);
}

#[test]
fn cursor_motion_clears_the_pending_wrap() {
    term!(t, 5, 3);
    t.write(b"abcde\x1b[DX");
    assert_eq!(screen(&t), ["abcXe", "", ""]);
    t.write(b"\x1b[1;5Hz\x1b[1;1HY");
    assert_eq!(line(&t, 0), "YbcXz");
}

#[test]
fn backspace_after_pending_wrap_moves_from_the_last_column() {
    term!(t, 5, 2);
    t.write(b"abcde\x08X");
    assert_eq!(screen(&t), ["abcXe", ""]);
}

#[test]
fn autowrap_off_overwrites_the_last_column() {
    term!(t, 5, 2);
    t.write(b"\x1b[?7labcdefg");
    assert_eq!(screen(&t), ["abcdg", ""]);
    assert_eq!(pos(&t), (0, 4));
    t.write("é".as_bytes());
    assert_eq!(line(&t, 0), "abcdé");
    t.write(b"\x1b[?7hxy");
    assert_eq!(screen(&t), ["abcdx", "y"]);
}

#[test]
fn disabling_autowrap_cancels_a_pending_wrap() {
    term!(t, 3, 2);
    t.write(b"abc\x1b[?7lX");
    assert_eq!(screen(&t), ["abX", ""]);
}

#[test]
fn bottom_right_corner_scrolls_only_on_the_next_character() {
    term!(t, 3, 2);
    t.write(b"abc\r\ndef");
    assert_eq!(screen(&t), ["abc", "def"]);
    assert_eq!(pos(&t), (1, 2));
    t.write(b"g");
    assert_eq!(screen(&t), ["def", "g"]);
    assert_eq!(pos(&t), (1, 1));
}

#[test]
fn one_write_longer_than_the_grid_wraps_and_scrolls() {
    term!(t, 3, 2);
    t.write(b"abcdefghijklmnopqrst");
    assert_eq!(screen(&t), ["pqr", "st"]);
    term!(u, 3, 2);
    u.write("αβγδεζηθικλμνξοπρστ".as_bytes());
    assert_eq!(screen(&u), ["πρσ", "τ"]);
}

#[test]
fn wrap_on_a_one_column_terminal() {
    term!(t, 1, 3);
    t.write(b"abcd");
    assert_eq!(screen(&t), ["b", "c", "d"]);
    assert_eq!(pos(&t), (2, 0));
}

// ---- controles C0 ----

#[test]
fn backspace_stops_at_column_zero() {
    term!(t, 5, 2);
    t.write(b"\x08\x08ab\x08X");
    assert_eq!(line(&t, 0), "aX");
    assert_eq!(pos(&t), (0, 2));
}

#[test]
fn tab_moves_to_the_next_multiple_of_eight() {
    term!(t, 20, 2);
    t.write(b"a\tb");
    assert_eq!(at(&t, 0, 8).ch, 'b');
    t.write(b"\t");
    assert_eq!(pos(&t), (0, 16));
    t.write(b"\r\t");
    assert_eq!(pos(&t), (0, 8));
}

#[test]
fn tab_clamps_to_the_last_column() {
    term!(t, 10, 2);
    t.write(b"\t\t\t");
    assert_eq!(pos(&t), (0, 9));
}

#[test]
fn line_feed_vt_and_ff_move_down_keeping_the_column() {
    term!(t, 6, 4);
    t.write(b"ab\x0bc\x0cd\ne");
    assert_eq!(screen(&t), ["ab", "  c", "   d", "    e"]);
}

#[test]
fn line_feed_at_the_bottom_scrolls() {
    term!(t, 4, 2);
    t.write(b"a\r\nb\r\nc");
    assert_eq!(screen(&t), ["b", "c"]);
    assert_eq!(pos(&t), (1, 1));
}

#[test]
fn carriage_return_goes_to_column_zero() {
    term!(t, 6, 2);
    t.write(b"abc\rX");
    assert_eq!(line(&t, 0), "Xbc");
}

#[test]
fn bell_is_reported_once() {
    term!(t, 4, 2);
    assert!(!t.take_bell());
    t.write(b"a\x07\x07b");
    assert_eq!(line(&t, 0), "ab");
    assert!(t.take_bell());
    assert!(!t.take_bell());
}

#[test]
fn unsupported_controls_are_ignored() {
    term!(t, 6, 2);
    t.write(b"a\x00\x01\x05\x0e\x0f\x7fb");
    assert_eq!(line(&t, 0), "ab");
}

// ---- ESC ----

#[test]
fn decsc_and_decrc_restore_position_and_attributes() {
    term!(t, 10, 10);
    t.write(b"\x1b[5;5H\x1b[31m\x1b7\x1b[1;1H\x1b[0m\x1b8X");
    let cell = at(&t, 4, 4);
    assert_eq!(cell.ch, 'X');
    assert_eq!(cell.fg, 1);
    assert!(!cell.flags.contains(Flags::DEFAULT_FG));
    assert_eq!(pos(&t), (4, 5));
}

#[test]
fn decrc_without_a_save_goes_home_with_defaults() {
    term!(t, 10, 5);
    t.write(b"\x1b[3;3H\x1b[31m\x1b8A");
    assert_eq!(at(&t, 0, 0).ch, 'A');
    assert!(at(&t, 0, 0).flags.contains(Flags::DEFAULT_FG));
}

#[test]
fn csi_s_and_u_share_the_save_slot_with_decsc() {
    term!(t, 10, 5);
    t.write(b"\x1b[2;3H\x1b[s\x1b[H\x1b[u");
    assert_eq!(pos(&t), (1, 2));
    t.write(b"\x1b[4;4H\x1b7\x1b[H\x1b[u");
    assert_eq!(pos(&t), (3, 3));
}

#[test]
fn decrc_restores_the_wrap_flag_it_saved() {
    term!(t, 3, 3);
    t.write(b"\x1b7abc\x1b8X");
    assert_eq!(screen(&t), ["Xbc", "", ""]);
    term!(u, 3, 3);
    u.write(b"abc\x1b7\x1b[3;1H\x1b8Y");
    assert_eq!(screen(&u), ["abc", "Y", ""]);
}

#[test]
fn ind_moves_down_and_scrolls_at_the_bottom() {
    term!(t, 5, 3);
    t.write(b"ab\x1bDc");
    assert_eq!(screen(&t), ["ab", "  c", ""]);
    t.write(b"\x1bD\x1bDd");
    assert_eq!(screen(&t), ["  c", "", "   d"]);
}

#[test]
fn nel_moves_to_the_start_of_the_next_line() {
    term!(t, 5, 3);
    t.write(b"ab\x1bEc");
    assert_eq!(screen(&t), ["ab", "c", ""]);
}

#[test]
fn ri_moves_up_and_scrolls_down_at_the_top() {
    term!(t, 5, 3);
    t.write(b"a\r\nb\r\nc\x1bMX");
    assert_eq!(screen(&t), ["a", "bX", "c"]);
    t.write(b"\x1b[H\x1bM");
    assert_eq!(screen(&t), ["", "a", "bX"]);
    assert_eq!(pos(&t), (0, 0));
    t.write(b"\x1bM");
    assert_eq!(screen(&t), ["", "", "a"]);
}

#[test]
fn ris_resets_screen_cursor_pen_region_and_modes() {
    term!(t, 6, 4);
    t.write(b"\x1b[1;31mabc\x1b[2;3r\x1b[?1;25;7l\x1b[?6h\x1b[2;2H\x1b7\x1bc");
    assert_eq!(screen(&t), ["", "", "", ""]);
    assert_eq!(pos(&t), (0, 0));
    assert!(t.cursor().visible);
    t.write(b"x\x1b[4;1H\n");
    assert_eq!(
        at(&t, 0, 0).flags,
        Flags::DEFAULT_FG.union(Flags::DEFAULT_BG)
    );
    assert_eq!(screen(&t), ["", "", "", ""]);
    t.write(b"abcdefg");
    assert_eq!(line(&t, 3), "g");
    let mut out = [0u8; 8];
    assert_eq!(t.encode_key(Key::Up, &mut out), 3);
    assert_eq!(&out[..3], b"\x1b[A");
    t.write(b"\x1b8");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn ris_in_the_middle_of_a_scrolled_screen_clears_the_ring() {
    term!(t, 4, 3);
    numbered(&mut t, 8);
    t.write(b"\x1bcX");
    assert_eq!(screen(&t), ["X", "", ""]);
}

#[test]
fn unknown_escapes_and_charset_selection_are_ignored() {
    term!(t, 6, 2);
    t.write(b"\x1b(0A\x1b)B\x1b#8B\x1b=\x1b>\x1bHC\x1b%GD");
    assert_eq!(line(&t, 0), "ABCD");
}

// ---- movimento CSI ----

#[test]
fn cursor_movement_defaults_to_one_and_treats_zero_as_one() {
    term!(t, 10, 6);
    t.write(b"\x1b[3;3H\x1b[A");
    assert_eq!(pos(&t), (1, 2));
    t.write(b"\x1b[B\x1b[C\x1b[D");
    assert_eq!(pos(&t), (2, 2));
    t.write(b"\x1b[0A\x1b[0B\x1b[0C\x1b[0D");
    assert_eq!(pos(&t), (2, 2));
    t.write(b"\x1b[2A\x1b[3C");
    assert_eq!(pos(&t), (0, 5));
}

#[test]
fn cursor_movement_clamps_at_the_edges() {
    term!(t, 10, 6);
    t.write(b"\x1b[3;3H\x1b[99A");
    assert_eq!(pos(&t), (0, 2));
    t.write(b"\x1b[99B");
    assert_eq!(pos(&t), (5, 2));
    t.write(b"\x1b[99C");
    assert_eq!(pos(&t), (5, 9));
    t.write(b"\x1b[99D");
    assert_eq!(pos(&t), (5, 0));
}

#[test]
fn huge_parameters_saturate_without_overflow() {
    term!(t, 10, 6);
    let big = "99999999999999999999999999";
    t.write(format!("\x1b[{big};{big}H").as_bytes());
    assert_eq!(pos(&t), (5, 9));
    t.write(
        format!("\x1b[{big}A\x1b[{big}D\x1b[{big}S\x1b[{big}L\x1b[{big}@\x1b[{big}P").as_bytes(),
    );
    t.write(
        format!("\x1b[{big}X\x1b[{big}T\x1b[{big}M\x1b[{big}b\x1b[{big}d\x1b[{big}G").as_bytes(),
    );
    assert!(t.cursor().row < 6 && t.cursor().col < 10);
}

#[test]
fn cnl_and_cpl_move_lines_and_reset_the_column() {
    term!(t, 10, 6);
    t.write(b"\x1b[2;5H\x1b[2E");
    assert_eq!(pos(&t), (3, 0));
    t.write(b"\x1b[3;5H\x1b[F");
    assert_eq!(pos(&t), (1, 0));
}

#[test]
fn cha_and_vpa_set_one_axis() {
    term!(t, 10, 6);
    t.write(b"\x1b[3;3H\x1b[7G");
    assert_eq!(pos(&t), (2, 6));
    t.write(b"\x1b[5d");
    assert_eq!(pos(&t), (4, 6));
    t.write(b"\x1b[G\x1b[d");
    assert_eq!(pos(&t), (0, 0));
    t.write(b"\x1b[99G\x1b[99d");
    assert_eq!(pos(&t), (5, 9));
}

#[test]
fn hpa_hpr_and_vpr_are_aliases() {
    term!(t, 10, 6);
    t.write(b"\x1b[3`");
    assert_eq!(pos(&t), (0, 2));
    t.write(b"\x1b[2a\x1b[2e");
    assert_eq!(pos(&t), (2, 4));
}

#[test]
fn cup_and_hvp_are_one_based_with_defaults() {
    term!(t, 10, 6);
    t.write(b"\x1b[2;3H");
    assert_eq!(pos(&t), (1, 2));
    t.write(b"\x1b[;5H");
    assert_eq!(pos(&t), (0, 4));
    t.write(b"\x1b[5;H");
    assert_eq!(pos(&t), (4, 0));
    t.write(b"\x1b[3;4f");
    assert_eq!(pos(&t), (2, 3));
    t.write(b"\x1b[H");
    assert_eq!(pos(&t), (0, 0));
    t.write(b"\x1b[0;0H");
    assert_eq!(pos(&t), (0, 0));
    t.write(b"\x1b[99;99H");
    assert_eq!(pos(&t), (5, 9));
}

#[test]
fn vertical_movement_stops_at_the_scroll_margins() {
    term!(t, 6, 10);
    t.write(b"\x1b[3;5r\x1b[4;1H\x1b[9A");
    assert_eq!(pos(&t), (2, 0));
    t.write(b"\x1b[9B");
    assert_eq!(pos(&t), (4, 0));
    t.write(b"\x1b[1;1H\x1b[9B");
    assert_eq!(pos(&t), (4, 0));
    t.write(b"\x1b[8;1H\x1b[9B");
    assert_eq!(pos(&t), (9, 0));
    t.write(b"\x1b[1;1H\x1b[9A");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn origin_mode_makes_addressing_relative_to_the_region() {
    term!(t, 6, 8);
    t.write(b"\x1b[3;6r\x1b[?6h");
    assert_eq!(pos(&t), (2, 0));
    t.write(b"\x1b[1;1H");
    assert_eq!(pos(&t), (2, 0));
    t.write(b"\x1b[99;2H");
    assert_eq!(pos(&t), (5, 1));
    t.write(b"\x1b[2;1H\x1b[6n");
    assert_eq!(reply(&mut t), b"\x1b[2;1R");
    t.write(b"\x1b[4d");
    assert_eq!(t.cursor().row, 5);
    t.write(b"\x1b[?6l");
    assert_eq!(pos(&t), (0, 0));
    t.write(b"\x1b[1;1H");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn decsc_saves_the_origin_mode() {
    term!(t, 6, 8);
    t.write(b"\x1b[3;6r\x1b[?6h\x1b7\x1b[?6l\x1b8\x1b[1;1H");
    assert_eq!(pos(&t), (2, 0));
}

// ---- edição ----

fn filled(t: &mut Terminal<'_>) {
    t.write(b"aaaaa\r\nbbbbb\r\nccccc");
}

#[test]
fn ed_zero_erases_from_the_cursor_to_the_end() {
    term!(t, 5, 3);
    filled(&mut t);
    t.write(b"\x1b[2;3H\x1b[J");
    assert_eq!(screen(&t), ["aaaaa", "bb", ""]);
    t.write(b"\x1b[1;4H\x1b[0J");
    assert_eq!(screen(&t), ["aaa", "", ""]);
}

#[test]
fn ed_one_erases_from_the_start_through_the_cursor() {
    term!(t, 5, 3);
    filled(&mut t);
    t.write(b"\x1b[2;3H\x1b[1J");
    assert_eq!(screen(&t), ["", "   bb", "ccccc"]);
}

#[test]
fn ed_two_clears_everything_and_keeps_the_cursor() {
    term!(t, 5, 3);
    filled(&mut t);
    t.write(b"\x1b[2;2H\x1b[2J");
    assert_eq!(screen(&t), ["", "", ""]);
    assert_eq!(pos(&t), (1, 1));
}

#[test]
fn ed_three_and_unknown_modes_do_nothing() {
    term!(t, 5, 3);
    filled(&mut t);
    t.write(b"\x1b[3J\x1b[9J");
    assert_eq!(screen(&t), ["aaaaa", "bbbbb", "ccccc"]);
}

#[test]
fn el_erases_by_mode() {
    term!(t, 6, 3);
    t.write(b"abcdef\x1b[1;3H\x1b[K");
    assert_eq!(line(&t, 0), "ab");
    t.write(b"\x1b[2;1Habcdef\x1b[2;3H\x1b[1K");
    assert_eq!(line(&t, 1), "   def");
    t.write(b"\x1b[3;1Habcdef\x1b[3;3H\x1b[2K");
    assert_eq!(line(&t, 2), "");
    assert_eq!(pos(&t), (2, 2));
    t.write(b"\x1b[1;1H\x1b[7K");
    assert_eq!(line(&t, 0), "ab");
}

#[test]
fn erase_uses_the_current_background_and_drops_other_attributes() {
    term!(t, 4, 2);
    t.write(b"\x1b[1;31;44m\x1b[2J");
    for row in 0..2 {
        for col in 0..4 {
            let cell = at(&t, row, col);
            assert_eq!(cell.ch, ' ');
            assert_eq!(cell.bg, 4);
            assert!(!cell.flags.contains(Flags::DEFAULT_BG));
            assert!(!cell.flags.contains(Flags::BOLD));
            assert!(cell.flags.contains(Flags::DEFAULT_FG));
        }
    }
    t.write(b"\x1b[0m\x1b[K");
    assert_eq!(at(&t, 0, 0).bg, 0);
    assert!(at(&t, 0, 0).flags.contains(Flags::DEFAULT_BG));
}

#[test]
fn every_erasing_sequence_honors_the_background() {
    term!(t, 6, 3);
    t.write(b"abcdef\x1b[44m\x1b[1;2H\x1b[2X\x1b[1;5H\x1b[K");
    assert_eq!(at(&t, 0, 1).bg, 4);
    assert_eq!(at(&t, 0, 2).bg, 4);
    assert_eq!(at(&t, 0, 3).bg, 0);
    assert_eq!(at(&t, 0, 5).bg, 4);
    t.write(b"\x1b[1;1H\x1b[P\x1b[@");
    assert_eq!(at(&t, 0, 0).bg, 4);
    assert_eq!(at(&t, 0, 5).bg, 4);
    t.write(b"\x1b[1;1H\x1b[L");
    assert_eq!(at(&t, 0, 3).bg, 4);
}

#[test]
fn il_inserts_blank_lines_at_the_cursor() {
    term!(t, 3, 5);
    numbered(&mut t, 5);
    t.write(b"\x1b[2;3H\x1b[L");
    assert_eq!(screen(&t), ["1", "", "2", "3", "4"]);
    assert_eq!(pos(&t), (1, 0));
    t.write(b"\x1b[99L");
    assert_eq!(screen(&t), ["1", "", "", "", ""]);
}

#[test]
fn dl_deletes_lines_at_the_cursor() {
    term!(t, 3, 5);
    numbered(&mut t, 5);
    t.write(b"\x1b[2;2H\x1b[M");
    assert_eq!(screen(&t), ["1", "3", "4", "5", ""]);
    assert_eq!(pos(&t), (1, 0));
    t.write(b"\x1b[2M");
    assert_eq!(screen(&t), ["1", "5", "", "", ""]);
    t.write(b"\x1b[99M");
    assert_eq!(screen(&t), ["1", "", "", "", ""]);
}

#[test]
fn il_and_dl_work_inside_the_region_only() {
    term!(t, 3, 5);
    numbered(&mut t, 5);
    t.write(b"\x1b[2;4r\x1b[2;1H\x1b[L");
    assert_eq!(screen(&t), ["1", "", "2", "3", "5"]);
    t.write(b"\x1b[M\x1b[M");
    assert_eq!(screen(&t), ["1", "3", "", "", "5"]);
}

#[test]
fn il_and_dl_outside_the_region_do_nothing() {
    term!(t, 3, 5);
    numbered(&mut t, 5);
    t.write(b"\x1b[2;4r\x1b[1;3H\x1b[L\x1b[M\x1b[5;2H\x1b[L\x1b[M");
    assert_eq!(screen(&t), ["1", "2", "3", "4", "5"]);
    assert_eq!(pos(&t), (4, 1));
}

#[test]
fn ich_shifts_the_rest_of_the_line_right() {
    term!(t, 8, 2);
    t.write(b"abcdef\x1b[1;3H\x1b[2@");
    assert_eq!(line(&t, 0), "ab  cdef");
    assert_eq!(pos(&t), (0, 2));
    t.write(b"\x1b[99@");
    assert_eq!(line(&t, 0), "ab");
}

#[test]
fn dch_shifts_the_rest_of_the_line_left() {
    term!(t, 8, 2);
    t.write(b"abcdef\x1b[1;2H\x1b[2P");
    assert_eq!(line(&t, 0), "adef");
    t.write(b"\x1b[99P");
    assert_eq!(line(&t, 0), "a");
}

#[test]
fn ech_blanks_without_moving_anything() {
    term!(t, 8, 2);
    t.write(b"abcdef\x1b[1;2H\x1b[2X");
    assert_eq!(line(&t, 0), "a  def");
    assert_eq!(pos(&t), (0, 1));
    t.write(b"\x1b[99X");
    assert_eq!(line(&t, 0), "a");
}

#[test]
fn su_and_sd_scroll_without_moving_the_cursor() {
    term!(t, 3, 5);
    numbered(&mut t, 5);
    t.write(b"\x1b[3;2H\x1b[2S");
    assert_eq!(screen(&t), ["3", "4", "5", "", ""]);
    assert_eq!(pos(&t), (2, 1));
    t.write(b"\x1b[2T");
    assert_eq!(screen(&t), ["", "", "3", "4", "5"]);
    t.write(b"\x1b[S\x1b[T");
    assert_eq!(screen(&t), ["", "", "3", "4", "5"]);
}

#[test]
fn su_and_sd_larger_than_the_region_clear_it() {
    term!(t, 3, 4);
    numbered(&mut t, 4);
    t.write(b"\x1b[2;3r\x1b[99S");
    assert_eq!(screen(&t), ["1", "", "", "4"]);
    t.write(b"\x1b[1;1H\x1b[99T");
    assert_eq!(screen(&t), ["1", "", "", "4"]);
    t.write(b"\x1b[r\x1b[99S");
    assert_eq!(screen(&t), ["", "", "", ""]);
}

#[test]
fn su_and_sd_respect_the_region() {
    term!(t, 3, 5);
    numbered(&mut t, 5);
    t.write(b"\x1b[2;4r\x1b[S");
    assert_eq!(screen(&t), ["1", "3", "4", "", "5"]);
    t.write(b"\x1b[T");
    assert_eq!(screen(&t), ["1", "", "3", "4", "5"]);
}

#[test]
fn rep_repeats_the_last_printed_character() {
    term!(t, 10, 2);
    t.write(b"a\x1b[3b");
    assert_eq!(line(&t, 0), "aaaa");
    t.write(b"\x1bcz\x1b[2b");
    assert_eq!(line(&t, 0), "zzz");
    t.write(b"\x1bc\x1b[5b");
    assert_eq!(line(&t, 0), "");
}

// ---- região de rolagem ----

#[test]
fn decstbm_scrolls_only_inside_the_region_and_homes_the_cursor() {
    term!(t, 3, 5);
    numbered(&mut t, 5);
    t.write(b"\x1b[2;4r");
    assert_eq!(pos(&t), (0, 0));
    t.write(b"\x1b[4;1H\n");
    assert_eq!(screen(&t), ["1", "3", "4", "", "5"]);
    assert_eq!(pos(&t), (3, 0));
}

#[test]
fn line_feed_outside_the_region_never_scrolls_it() {
    term!(t, 3, 5);
    numbered(&mut t, 5);
    t.write(b"\x1b[2;3r\x1b[5;1H\n\n");
    assert_eq!(screen(&t), ["1", "2", "3", "4", "5"]);
    assert_eq!(pos(&t), (4, 0));
    t.write(b"\x1b[4;1H\n");
    assert_eq!(pos(&t), (4, 0));
    t.write(b"\x1b[1;1H\n\n");
    assert_eq!(pos(&t), (2, 0));
    assert_eq!(screen(&t), ["1", "2", "3", "4", "5"]);
}

#[test]
fn decstbm_without_parameters_restores_the_full_screen() {
    term!(t, 3, 4);
    numbered(&mut t, 4);
    t.write(b"\x1b[2;3r\x1b[r\x1b[4;1H\n");
    assert_eq!(screen(&t), ["2", "3", "4", ""]);
}

#[test]
fn decstbm_with_a_one_line_or_inverted_region_is_ignored() {
    term!(t, 3, 4);
    numbered(&mut t, 4);
    t.write(b"\x1b[2;2r\x1b[3;1H");
    assert_eq!(pos(&t), (2, 0));
    t.write(b"\x1b[4;1H\n");
    assert_eq!(screen(&t), ["2", "3", "4", ""]);
    t.write(b"\x1b[3;2r\x1b[4;1H\n");
    assert_eq!(screen(&t), ["3", "4", "", ""]);
    t.write(b"\x1b[3;1H\x1b[9;9r");
    assert_eq!(pos(&t), (2, 0));
    t.write(b"\x1b[0;0r");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn decstbm_bottom_beyond_the_screen_is_clamped() {
    term!(t, 3, 4);
    numbered(&mut t, 4);
    t.write(b"\x1b[2;99r\x1b[4;1H\n");
    assert_eq!(screen(&t), ["1", "3", "4", ""]);
    t.write(b"\x1b[3r\x1b[4;1H\n");
    assert_eq!(screen(&t), ["1", "3", "", ""]);
}

#[test]
fn ri_at_the_top_margin_scrolls_the_region_down() {
    term!(t, 3, 5);
    numbered(&mut t, 5);
    t.write(b"\x1b[2;4r\x1b[2;1H\x1bM");
    assert_eq!(screen(&t), ["1", "", "2", "3", "5"]);
    t.write(b"\x1b[1;1H\x1bM");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn a_one_row_terminal_scrolls_in_place() {
    term!(t, 4, 1);
    t.write(b"ab\ncd");
    assert_eq!(screen(&t), ["  cd"]);
    t.write(b"\x1bM\x1b[L\x1b[M\x1b[S\x1b[T\x1b[2;3r");
    assert_eq!(screen(&t), [""]);
    assert_eq!(pos(&t), (0, 0));
    t.write(b"x\ny");
    assert_eq!(screen(&t), [" y"]);
}

// ---- SGR ----

#[test]
fn sgr_attributes_turn_on_and_off() {
    term!(t, 10, 2);
    t.write(b"\x1b[1;2;3;4;7;9mX");
    let on = Flags::BOLD
        .union(Flags::DIM)
        .union(Flags::ITALIC)
        .union(Flags::UNDERLINE)
        .union(Flags::INVERSE)
        .union(Flags::STRIKE);
    assert!(at(&t, 0, 0).flags.contains(on));
    t.write(b"\x1b[22;23;24;27;29mY");
    assert_eq!(at(&t, 0, 1).flags, Cell::BLANK.flags);
}

#[test]
fn sgr_22_clears_bold_and_dim_together() {
    term!(t, 10, 2);
    t.write(b"\x1b[1;2;3m\x1b[22mX");
    assert_eq!(at(&t, 0, 0).flags, Flags::ITALIC.union(Cell::BLANK.flags));
}

#[test]
fn sgr_standard_and_bright_colors_map_to_indices() {
    term!(t, 20, 2);
    for (code, fg) in [(30, 0), (31, 1), (37, 7), (90, 8), (97, 15)] {
        t.write(format!("\x1b[{code}mX").as_bytes());
        let cell = at(&t, 0, t.cursor().col - 1);
        assert_eq!(cell.fg, fg);
        assert!(!cell.flags.contains(Flags::DEFAULT_FG));
    }
    for (code, bg) in [(40, 0), (44, 4), (47, 7), (100, 8), (107, 15)] {
        t.write(format!("\x1b[{code}mY").as_bytes());
        let cell = at(&t, 0, t.cursor().col - 1);
        assert_eq!(cell.bg, bg);
        assert!(!cell.flags.contains(Flags::DEFAULT_BG));
    }
}

#[test]
fn sgr_256_colors_use_the_index_directly() {
    term!(t, 10, 2);
    t.write(b"\x1b[38;5;200;48;5;17mX");
    assert_eq!((at(&t, 0, 0).fg, at(&t, 0, 0).bg), (200, 17));
    t.write(b"\x1b[38;5;0;48;5;255mY");
    assert_eq!((at(&t, 0, 1).fg, at(&t, 0, 1).bg), (0, 255));
    t.write(b"\x1b[38;5;256mZ\x1b[48;5;999mW");
    assert_eq!((at(&t, 0, 2).fg, at(&t, 0, 3).bg), (0, 255));
}

#[test]
fn sgr_truecolor_maps_to_the_nearest_palette_index() {
    term!(t, 10, 2);
    t.write(b"\x1b[38;2;255;0;0;48;2;0;0;255mX");
    assert_eq!((at(&t, 0, 0).fg, at(&t, 0, 0).bg), (196, 21));
    t.write(b"\x1b[38;2;128;128;128mY\x1b[38;2;999;0;0mZ");
    assert_eq!(at(&t, 0, 1).fg, 244);
    assert_eq!(at(&t, 0, 2).fg, 196);
}

#[test]
fn sgr_default_colors_switch_back_to_the_theme() {
    term!(t, 10, 2);
    t.write(b"\x1b[31;42m\x1b[39;49mX");
    assert!(
        at(&t, 0, 0)
            .flags
            .contains(Flags::DEFAULT_FG.union(Flags::DEFAULT_BG))
    );
    t.write(b"\x1b[32mY");
    assert!(!at(&t, 0, 1).flags.contains(Flags::DEFAULT_FG));
    assert!(at(&t, 0, 1).flags.contains(Flags::DEFAULT_BG));
}

#[test]
fn sgr_reset_has_several_spellings() {
    for reset in ["\x1b[0m", "\x1b[m", "\x1b[;m", "\x1b[00m"] {
        term!(t, 10, 2);
        t.write(format!("\x1b[1;31;44m{reset}X").as_bytes());
        assert_eq!(
            at(&t, 0, 0),
            Cell {
                ch: 'X',
                ..Cell::BLANK
            },
            "{reset:?}"
        );
    }
}

#[test]
fn bold_does_not_change_the_color_index() {
    term!(t, 10, 2);
    t.write(b"\x1b[1;31mX\x1b[1;37mY\x1b[0;1mZ");
    assert_eq!(at(&t, 0, 0).fg, 1);
    assert_eq!(at(&t, 0, 1).fg, 7);
    assert!(
        at(&t, 0, 2)
            .flags
            .contains(Flags::BOLD.union(Flags::DEFAULT_FG))
    );
}

#[test]
fn incomplete_extended_colors_are_ignored() {
    term!(t, 10, 2);
    for seq in [
        "\x1b[38m",
        "\x1b[38;5m",
        "\x1b[38;2m",
        "\x1b[38;2;1;2m",
        "\x1b[48;9;1;2;3m",
        "\x1b[38;",
    ] {
        t.write(seq.as_bytes());
        t.write(b"X");
        let cell = at(&t, 0, t.cursor().col - 1);
        assert!(cell.flags.contains(Flags::DEFAULT_FG), "{seq:?}");
        t.write(b"\x1b[m");
    }
}

#[test]
fn sgr_continues_after_an_extended_color() {
    term!(t, 10, 2);
    t.write(b"\x1b[38;5;9;1;4mX\x1b[48;2;0;0;255;3mY");
    assert_eq!(at(&t, 0, 0).fg, 9);
    assert!(
        at(&t, 0, 0)
            .flags
            .contains(Flags::BOLD.union(Flags::UNDERLINE))
    );
    assert_eq!(at(&t, 0, 1).bg, 21);
    assert!(at(&t, 0, 1).flags.contains(Flags::ITALIC));
}

#[test]
fn unsupported_sgr_codes_are_ignored() {
    term!(t, 10, 2);
    t.write(b"\x1b[5;8;21;53;1000;65535;65536mX");
    assert_eq!(
        at(&t, 0, 0),
        Cell {
            ch: 'X',
            ..Cell::BLANK
        }
    );
}

#[test]
fn sgr_with_more_parameters_than_the_limit_does_not_panic() {
    term!(t, 10, 2);
    let mut seq = String::from("\x1b[");
    for _ in 0..100 {
        seq.push_str("1;");
    }
    seq.push_str("31mX");
    t.write(seq.as_bytes());
    assert!(at(&t, 0, 0).flags.contains(Flags::BOLD));
}

#[test]
fn utf8_characters_take_the_current_pen() {
    term!(t, 10, 2);
    t.write("\x1b[32mé".as_bytes());
    assert_eq!(at(&t, 0, 0).fg, 2);
}

// ---- respostas ----

#[test]
fn dsr_five_reports_ok() {
    term!(t, 10, 5);
    t.write(b"\x1b[5n");
    assert_eq!(reply(&mut t), b"\x1b[0n");
    assert_eq!(reply(&mut t).len(), 0);
}

#[test]
fn dsr_six_reports_the_cursor_one_based() {
    term!(t, 100, 30);
    t.write(b"\x1b[3;7H\x1b[6n");
    assert_eq!(reply(&mut t), b"\x1b[3;7R");
    t.write(b"\x1b[30;100H\x1b[6n");
    assert_eq!(reply(&mut t), b"\x1b[30;100R");
}

#[test]
fn dsr_six_with_a_pending_wrap_reports_the_last_column() {
    term!(t, 5, 2);
    t.write(b"abcde\x1b[6n");
    assert_eq!(reply(&mut t), b"\x1b[1;5R");
}

#[test]
fn dsr_with_other_parameters_is_ignored() {
    term!(t, 5, 2);
    t.write(b"\x1b[7n\x1b[n\x1b[?6n\x1b[?5n");
    assert_eq!(reply(&mut t).len(), 0);
}

#[test]
fn da_answers_as_a_vt102() {
    term!(t, 5, 2);
    t.write(b"\x1b[c");
    assert_eq!(reply(&mut t), b"\x1b[?6c");
    t.write(b"\x1b[0c");
    assert_eq!(reply(&mut t), b"\x1b[?6c");
    t.write(b"\x1b[1c\x1b[>c\x1b[>0c\x1b[=c");
    assert_eq!(reply(&mut t).len(), 0);
}

#[test]
fn replies_queue_in_order() {
    term!(t, 5, 2);
    t.write(b"\x1b[5n\x1b[c\x1b[6n");
    assert_eq!(reply(&mut t), b"\x1b[0n\x1b[?6c\x1b[1;1R");
}

#[test]
fn take_reply_drains_in_small_pieces() {
    term!(t, 5, 2);
    t.write(b"\x1b[5n\x1b[5n");
    let mut small = [0u8; 3];
    assert_eq!(t.take_reply(&mut small), 3);
    assert_eq!(&small, b"\x1b[0");
    assert_eq!(t.take_reply(&mut small), 3);
    assert_eq!(&small, b"n\x1b[");
    assert_eq!(t.take_reply(&mut small), 2);
    assert_eq!(&small[..2], b"0n");
    assert_eq!(t.take_reply(&mut small), 0);
    assert_eq!(t.take_reply(&mut []), 0);
}

#[test]
fn take_reply_with_an_empty_buffer_keeps_the_queue() {
    term!(t, 5, 2);
    t.write(b"\x1b[5n");
    assert_eq!(t.take_reply(&mut []), 0);
    assert_eq!(reply(&mut t), b"\x1b[0n");
}

#[test]
fn replies_that_do_not_fit_are_dropped_whole() {
    term!(t, 5, 2);
    for _ in 0..40 {
        t.write(b"\x1b[5n");
    }
    assert_eq!(reply(&mut t).len(), REPLY_CAP);
    t.write(&b"\x1b[5n".repeat(15));
    t.write(b"\x1b[10;10H\x1b[6n");
    let out = reply(&mut t);
    assert_eq!(out.len() % 4, 0);
    assert!(out.len() <= REPLY_CAP);
}

// ---- modos ----

#[test]
fn decckm_changes_the_arrow_encoding() {
    term!(t, 5, 2);
    let mut out = [0u8; 8];
    let n = t.encode_key(Key::Up, &mut out);
    assert_eq!(&out[..n], b"\x1b[A");
    t.write(b"\x1b[?1h");
    let n = t.encode_key(Key::Up, &mut out);
    assert_eq!(&out[..n], b"\x1bOA");
    t.write(b"\x1b[?1l");
    let n = t.encode_key(Key::Left, &mut out);
    assert_eq!(&out[..n], b"\x1b[D");
}

#[test]
fn cursor_visibility_follows_dectcem() {
    term!(t, 5, 2);
    t.write(b"\x1b[?25l");
    assert!(!t.cursor().visible);
    t.write(b"\x1b[?25h");
    assert!(t.cursor().visible);
    t.write(b"\x1b[?1;25l");
    assert!(!t.cursor().visible);
}

#[test]
fn unsupported_private_modes_are_ignored() {
    term!(t, 5, 2);
    t.write(b"abc\x1b[?1049h\x1b[?47h\x1b[?1047h\x1b[?2004h\x1b[?1000h\x1b[?9999l");
    assert_eq!(screen(&t), ["abc", ""]);
    assert_eq!(pos(&t), (0, 3));
    t.write(b"\x1b[?1049lx");
    assert_eq!(line(&t, 0), "abcx");
}

#[test]
fn ansi_modes_are_ignored() {
    term!(t, 5, 2);
    t.write(b"ab\x1b[4h\x1b[1;1HX\x1b[4l\x1b[20h");
    assert_eq!(line(&t, 0), "Xb");
}

#[test]
fn csi_with_a_private_marker_is_not_taken_for_a_standard_one() {
    term!(t, 5, 2);
    t.write(b"\x1b[>4;2m\x1b[<1A\x1b[=1;1HX");
    assert_eq!(
        at(&t, 0, 0),
        Cell {
            ch: 'X',
            ..Cell::BLANK
        }
    );
    assert_eq!(line(&t, 0), "X");
    t.write(b"\x1b[?1mY\x1b[?2J");
    assert_eq!(line(&t, 0), "XY");
}

#[test]
fn csi_with_an_intermediate_is_not_taken_for_a_standard_one() {
    term!(t, 5, 2);
    t.write(b"ab\x1b[1;1H\x1b[1 @\x1b[2 q\x1b[!p\x1b[0\"q");
    assert_eq!(line(&t, 0), "ab");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn unknown_csi_finals_are_ignored() {
    term!(t, 5, 2);
    t.write(b"a\x1b[1;2;3z\x1b[99y\x1b[~b");
    assert_eq!(line(&t, 0), "ab");
}

// ---- strings, abortos e cortes ----

#[test]
fn osc_ended_by_bel_or_st_is_consumed() {
    term!(t, 10, 2);
    t.write(b"a\x1b]0;my title\x07b\x1b]2;other\x1b\\c");
    assert_eq!(line(&t, 0), "abc");
}

#[test]
fn osc_split_across_writes_is_still_consumed() {
    term!(t, 10, 2);
    t.write(b"a\x1b]0;ti");
    t.write(b"tle\x1b");
    t.write(b"\\b");
    assert_eq!(line(&t, 0), "ab");
}

#[test]
fn unterminated_osc_swallows_text_until_cancelled() {
    term!(t, 10, 2);
    t.write(b"a\x1b]0;lost\r\ntext");
    assert_eq!(line(&t, 0), "a");
    t.write(b"\x18b");
    assert_eq!(line(&t, 0), "ab");
}

#[test]
fn dcs_apc_pm_and_sos_are_consumed() {
    term!(t, 10, 2);
    t.write(b"a\x1bPqabc\x1b\\b\x1b_app\x1b\\c\x1b^pm\x1b\\d\x1bXsos\x1b\\e");
    assert_eq!(line(&t, 0), "abcde");
}

#[test]
fn csi_cut_between_writes_resumes() {
    term!(t, 10, 5);
    t.write(b"\x1b[3");
    t.write(b";4H");
    assert_eq!(pos(&t), (2, 3));
    t.write(b"\x1b");
    t.write(b"[");
    t.write(b"2J");
    t.write(b"\x1b[1;");
    t.write(b"1HX");
    assert_eq!(line(&t, 0), "X");
}

#[test]
fn can_and_sub_abort_a_sequence() {
    term!(t, 10, 2);
    t.write(b"ab\x1b[3\x18cd\x1b[4\x1aef");
    assert_eq!(line(&t, 0), "abcdef");
}

#[test]
fn esc_inside_a_sequence_starts_a_new_one() {
    term!(t, 10, 5);
    t.write(b"\x1b[1;2\x1b[3;4H");
    assert_eq!(pos(&t), (2, 3));
    t.write(b"\x1b\x1b[1;1H");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn c0_inside_csi_executes_and_the_sequence_goes_on() {
    term!(t, 10, 5);
    t.write(b"\x1b[2\n;3H");
    assert_eq!(pos(&t), (1, 2));
}

// ---- UTF-8 ----

#[test]
fn multibyte_characters_fill_one_cell_each() {
    term!(t, 10, 2);
    t.write("é€😀".as_bytes());
    assert_eq!(line(&t, 0), "é€😀");
    assert_eq!(pos(&t), (0, 3));
}

#[test]
fn utf8_cut_between_writes_is_reassembled_at_every_split() {
    let text = "a€😀é".as_bytes();
    for split in 1..text.len() {
        term!(t, 10, 2);
        t.write(&text[..split]);
        t.write(&text[split..]);
        assert_eq!(line(&t, 0), "a€😀é", "split at {split}");
    }
}

#[test]
fn invalid_utf8_prints_the_replacement_character() {
    term!(t, 10, 2);
    t.write(b"a\xffb\x80c\xc0\xafd\xf5e");
    assert_eq!(line(&t, 0), "a\u{FFFD}b\u{FFFD}c\u{FFFD}\u{FFFD}d\u{FFFD}e");
}

#[test]
fn truncated_utf8_is_replaced_and_the_following_byte_is_kept() {
    term!(t, 10, 2);
    t.write(&[0xE2, 0x82, b'A', 0xF0, 0x9F, 0x1B]);
    t.write(b"[2;2HZ");
    assert_eq!(line(&t, 0), "\u{FFFD}A\u{FFFD}");
    assert_eq!(at(&t, 1, 1).ch, 'Z');
}

#[test]
fn utf8_pending_at_the_end_of_a_write_stays_pending() {
    term!(t, 10, 2);
    t.write(&[0xE2, 0x82]);
    assert_eq!(pos(&t), (0, 0));
    t.write(&[0xAC]);
    assert_eq!(line(&t, 0), "€");
}

#[test]
fn c1_controls_in_utf8_are_ignored() {
    term!(t, 10, 2);
    t.write("a\u{85}b\u{9b}1;1H".as_bytes());
    assert_eq!(line(&t, 0), "ab1;1H");
}

#[test]
fn combining_and_zero_width_characters_are_dropped() {
    term!(t, 10, 2);
    t.write("e\u{301}x\u{200d}y\u{fe0f}z".as_bytes());
    assert_eq!(line(&t, 0), "exyz");
    assert_eq!(pos(&t), (0, 4));
}

#[test]
fn wide_characters_take_one_cell() {
    term!(t, 10, 2);
    t.write("日本語".as_bytes());
    assert_eq!(line(&t, 0), "日本語");
    assert_eq!(pos(&t), (0, 3));
}

#[test]
fn non_ascii_inside_a_sequence_is_ignored() {
    term!(t, 10, 5);
    t.write("\x1b[2é;3H".as_bytes());
    assert_eq!(pos(&t), (1, 2));
    t.write("\x1b]0;título\x07ok".as_bytes());
    assert_eq!(line(&t, 1), "  ok");
}

// ---- bloco contra byte a byte ----

fn script() -> Vec<u8> {
    let mut s = Vec::new();
    s.extend_from_slice(
        b"\x1b[2J\x1b[Hhello world, this is a long line that wraps around twice\r\n",
    );
    s.extend_from_slice("\x1b[1;31mcolor\x1b[0m é€😀 日本\u{301}\r\n".as_bytes());
    s.extend_from_slice(b"\x1b[3;6r\x1b[?6h\x1b[2;3H\x1b[44mregion\x1b[K\nA\nB\nC\nD\x1b[2L\x1b[M");
    s.extend_from_slice(
        b"\x1b[?6l\x1b[r\x1b[10;5H\x1b7\x1b[38;2;10;200;30mtruecolor\x1b8\x1b[@\x1b[2P\x1b[3X",
    );
    s.extend_from_slice(
        b"\x1b]0;title\x07\x1b[5n\x1b[6n\x1b[c\x1b[2S\x1b[T\x1bM\x1bD\x1bE\x1b[?25l\x1b[b",
    );
    for i in 0..40 {
        s.extend_from_slice(format!("line {i}\r\n").as_bytes());
    }
    s.extend_from_slice(b"\x1b[999;999Hcorner\x1b[?7lover\x1b[?7hx\x1b[1;1H\x1b[1J\x1b[2K\x1bc!");
    s
}

fn run_chunked(bytes: &[u8], sizes: &[usize]) -> ((Vec<Cell>, Cursor), Vec<u8>, Vec<usize>) {
    term!(t, 17, 9);
    let mut at = 0;
    let mut i = 0;
    while at < bytes.len() {
        let end = (at + sizes[i % sizes.len()]).min(bytes.len());
        t.write(&bytes[at..end]);
        at = end;
        i += 1;
    }
    let out = reply(&mut t);
    let rows = dirty(&mut t);
    (snapshot(&t), out, rows)
}

#[test]
fn byte_by_byte_and_block_writes_end_with_the_same_grid() {
    let bytes = script();
    let block = run_chunked(&bytes, &[bytes.len()]);
    assert_eq!(block, run_chunked(&bytes, &[1]));
    assert_eq!(block, run_chunked(&bytes, &[3, 1, 7, 2, 64]));
    assert_eq!(block, run_chunked(&bytes, &[5]));
}

#[test]
fn the_script_actually_exercises_the_screen() {
    let (grid, _, _) = run_chunked(&script(), &[1000]);
    assert!(grid.0.iter().any(|c| c.ch != ' '));
}

// ---- linhas sujas ----

#[test]
fn a_new_terminal_starts_fully_dirty() {
    term!(t, 5, 4);
    assert_eq!(dirty(&mut t), [0, 1, 2, 3]);
    assert_eq!(dirty(&mut t).len(), 0);
    assert!(t.take_dirty().is_empty());
}

#[test]
fn printing_marks_the_cursor_row() {
    term!(t, 5, 4);
    dirty(&mut t);
    t.write(b"x");
    assert_eq!(dirty(&mut t), [0]);
    t.write(b"yz\r\nw");
    assert_eq!(dirty(&mut t), [0, 1]);
}

#[test]
fn cursor_motion_marks_the_old_and_new_rows() {
    term!(t, 5, 4);
    dirty(&mut t);
    t.write(b"\x1b[4;3H");
    assert_eq!(dirty(&mut t), [0, 3]);
    t.write(b"\x1b[2;3H");
    assert_eq!(dirty(&mut t), [1, 3]);
}

#[test]
fn cursor_motion_inside_a_row_marks_that_row() {
    term!(t, 5, 4);
    dirty(&mut t);
    t.write(b"\x1b[1;5H");
    assert_eq!(dirty(&mut t), [0]);
}

#[test]
fn nothing_moved_means_nothing_dirty() {
    term!(t, 5, 4);
    dirty(&mut t);
    t.write(b"");
    t.write(b"\x1b]0;t\x07\x1b[5n\x07\x1b[31m\x1b[?1h");
    assert_eq!(dirty(&mut t).len(), 0);
    t.write(b"\x1b[3;3H\x1b[1;1H");
    assert_eq!(dirty(&mut t).len(), 0);
}

#[test]
fn hiding_or_showing_the_cursor_marks_its_row() {
    term!(t, 5, 4);
    t.write(b"\x1b[3;1H");
    dirty(&mut t);
    t.write(b"\x1b[?25l");
    assert_eq!(dirty(&mut t), [2]);
    t.write(b"\x1b[?25h");
    assert_eq!(dirty(&mut t), [2]);
}

#[test]
fn full_screen_scroll_marks_every_row() {
    term!(t, 5, 4);
    t.write(b"\x1b[4;1H");
    dirty(&mut t);
    t.write(b"\n");
    assert_eq!(dirty(&mut t), [0, 1, 2, 3]);
}

#[test]
fn region_scroll_marks_only_the_region() {
    term!(t, 5, 6);
    t.write(b"\x1b[2;4r\x1b[4;1H");
    dirty(&mut t);
    t.write(b"\n");
    assert_eq!(dirty(&mut t), [1, 2, 3]);
}

#[test]
fn line_and_character_edits_mark_the_rows_they_touch() {
    term!(t, 5, 6);
    t.write(b"\x1b[3;1H");
    dirty(&mut t);
    t.write(b"\x1b[K");
    assert_eq!(dirty(&mut t), [2]);
    t.write(b"\x1b[@\x1b[P\x1b[X");
    assert_eq!(dirty(&mut t), [2]);
    t.write(b"\x1b[J");
    assert_eq!(dirty(&mut t), [2, 3, 4, 5]);
    t.write(b"\x1b[1J");
    assert_eq!(dirty(&mut t), [0, 1, 2]);
    t.write(b"\x1b[2L");
    assert_eq!(dirty(&mut t), [2, 3, 4, 5]);
    t.write(b"\x1b[M");
    assert_eq!(dirty(&mut t), [2, 3, 4, 5]);
}

#[test]
fn ris_marks_every_row() {
    term!(t, 5, 4);
    dirty(&mut t);
    t.write(b"\x1bc");
    assert_eq!(dirty(&mut t), [0, 1, 2, 3]);
}

#[test]
fn dirty_tracking_reaches_rows_beyond_the_first_word() {
    term!(t, 4, 200);
    dirty(&mut t);
    t.write(b"\x1b[150;1Hx");
    assert_eq!(dirty(&mut t), [0, 149]);
    t.write(b"\x1b[200;1H\n");
    assert_eq!(dirty(&mut t).len(), 200);
}

// ---- anel de linhas ----

#[test]
fn many_scrolls_keep_the_logical_row_order() {
    term!(t, 4, 5);
    numbered(&mut t, 23);
    assert_eq!(screen(&t), ["19", "20", "21", "22", "23"]);
    t.write(b"\r\n24");
    assert_eq!(screen(&t), ["20", "21", "22", "23", "24"]);
}

#[test]
fn region_edits_after_the_ring_has_rotated_stay_correct() {
    term!(t, 4, 5);
    numbered(&mut t, 8);
    assert_eq!(screen(&t), ["4", "5", "6", "7", "8"]);
    t.write(b"\x1b[2;4r\x1b[2;1H\x1b[L");
    assert_eq!(screen(&t), ["4", "", "5", "6", "8"]);
    t.write(b"\x1b[M\x1b[M\x1b[T");
    assert_eq!(screen(&t), ["4", "", "6", "", "8"]);
    t.write(b"\x1b[r\x1b[T");
    assert_eq!(screen(&t), ["", "4", "", "6", ""]);
    t.write(b"\x1b[2S");
    assert_eq!(screen(&t), ["", "6", "", "", ""]);
    t.write(b"\x1b[4;1H\x1b[2M\x1b[2L");
    assert_eq!(screen(&t), ["", "6", "", "", ""]);
}

#[test]
fn scroll_down_rotates_the_ring_backwards() {
    term!(t, 4, 4);
    numbered(&mut t, 4);
    t.write(b"\x1b[H\x1bM\x1bM");
    assert_eq!(screen(&t), ["", "", "1", "2"]);
    t.write(b"\x1b[4;1H\n\n\n");
    assert_eq!(screen(&t), ["2", "", "", ""]);
    t.write(b"\x1b[3T");
    assert_eq!(screen(&t), ["", "", "", "2"]);
}

#[test]
fn scroll_up_beyond_a_region_that_starts_at_the_top_blanks_it() {
    term!(t, 4, 10);
    numbered(&mut t, 10);
    t.write(b"\x1b[1;5r\x1b[10S");
    assert_eq!(&screen(&t)[..6], ["", "", "", "", "", "6"]);
}

#[test]
fn delete_lines_beyond_a_region_that_starts_at_the_top_blanks_it() {
    term!(t, 4, 10);
    numbered(&mut t, 10);
    t.write(b"\x1b[1;5r\x1b[H\x1b[9M");
    assert_eq!(&screen(&t)[..6], ["", "", "", "", "", "6"]);
}

#[test]
fn erase_and_character_edits_work_on_a_rotated_ring() {
    term!(t, 6, 3);
    numbered(&mut t, 5);
    t.write(b"\x1b[1;1Habc\x1b[1;2H\x1b[P\x1b[3;1H\x1b[1K\x1b[2;1H\x1b[2K");
    assert_eq!(screen(&t), ["ac", "", ""]);
    t.write(b"\x1b[2J");
    assert_eq!(screen(&t), ["", "", ""]);
}

// ---- fuzz determinístico ----

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn byte(&mut self) -> u8 {
        const HOT: &[u8] = b"\x1b\x1b\x1b[[[]?;;0123456789ABCDEFGHIJKLMPSTXZ@`abcdfghlmnrsu\x07\x08\t\n\r\x0e\x0f\x18\x1a\\ \"!#()";
        const UTF8: &[u8] = &[
            0xC3, 0xA9, 0xE2, 0x82, 0xAC, 0xF0, 0x9F, 0x98, 0x80, 0xC2, 0x9B, 0xFF,
        ];
        let r = self.next();
        let pick = (r >> 8) as usize;
        match r % 10 {
            0..=1 => r.to_le_bytes()[2],
            2..=3 => UTF8[pick % UTF8.len()],
            4 => b"abcdefghijklmnopqrstuvwxyz"[pick % 26],
            _ => HOT[pick % HOT.len()],
        }
    }
}

fn check_invariants(t: &Terminal<'_>) {
    let cursor = t.cursor();
    assert!(cursor.row < t.rows() && cursor.col < t.cols(), "{cursor:?}");
    assert!(t.scroll_top <= t.scroll_bottom && t.scroll_bottom < t.rows());
    assert!(t.ring < t.rows());
    assert!(t.view <= t.history.len() && t.history.len() <= t.history.capacity());
    assert!(!t.alt_active || t.view == 0);
    for row in 0..t.rows() {
        assert_eq!(t.row(row).len(), t.cols());
    }
}

#[test]
fn random_bytes_never_panic_or_leave_the_grid() {
    for (seed, (cols, rows)) in [
        (1, (1, 1)),
        (2, (5, 3)),
        (3, (80, 24)),
        (4, (7, 256)),
        (5, (2, 40)),
    ] {
        let mut rng = XorShift(0x9E37_79B9_7F4A_7C15 ^ seed);
        term!(t, cols, rows);
        let mut fed = 0usize;
        let mut chunk = Vec::new();
        while fed < 300_000 {
            chunk.clear();
            let len = 1 + (rng.next() % 64) as usize;
            chunk.extend((0..len).map(|_| rng.byte()));
            t.write(&chunk);
            fed += len;
            check_invariants(&t);
            assert!(t.take_dirty().all(|row| row < rows));
            reply(&mut t);
            t.take_bell();
        }
    }
}

const MODES: &[u16] = &[1, 6, 7, 25, 47, 1047, 1048, 1049, 1049, 47];

fn well_formed_csi(rng: &mut XorShift, limit: u64) -> Vec<u8> {
    const FINALS: &[u8] = b"@ABCDEFGHIJKLMPSTXZ`abdefghlmnrsu";
    let mut seq = b"\x1b[".to_vec();
    let private = rng.next().is_multiple_of(4);
    if private {
        seq.push(b'?');
    }
    for index in 0..rng.next() % 4 {
        if index > 0 {
            seq.push(b';');
        }
        let value = if private && rng.next().is_multiple_of(2) {
            u64::from(MODES[usize::try_from(rng.next()).unwrap() % MODES.len()])
        } else {
            rng.next() % limit
        };
        seq.extend(value.to_string().bytes());
    }
    seq.push(FINALS[usize::try_from(rng.next()).unwrap() % FINALS.len()]);
    if rng.next().is_multiple_of(3) {
        seq.extend_from_slice(b"x\r\n");
    }
    seq
}

/// Sequências CSI bem formadas com parâmetros perto das bordas da grade: acham o
/// que o fuzz de bytes soltos raramente monta (região `1;Nr` seguida de `S`/`M`).
#[test]
fn random_well_formed_csi_never_panics() {
    for (seed, (cols, rows)) in [(6, (1, 1)), (7, (4, 10)), (8, (80, 24)), (9, (3, 256))] {
        let mut rng = XorShift(0x0123_4567_89AB_CDEF ^ seed);
        term!(t, cols, rows);
        let limit = rows.max(cols) as u64 + 3;
        for _ in 0..40_000 {
            t.write(&well_formed_csi(&mut rng, limit));
            check_invariants(&t);
            reply(&mut t);
        }
    }
}

const SCREEN_SEQUENCES: &[&[u8]] = &[
    b"\x1b[?1049h",
    b"\x1b[?1049l",
    b"\x1b[?47h",
    b"\x1b[?47l",
    b"\x1b[?1047h",
    b"\x1b[?1047l",
    b"\x1b[?1048h",
    b"\x1b[?1048l",
    b"\x1b[3J",
    b"\x1b[2J",
    b"\x1b(0",
    b"\x1b(B",
    b"\x1b)0",
    b"\x0e",
    b"\x0f",
    b"\x1bH",
    b"\x1b[3g",
    b"\x1b[g",
    b"\x1b[5I",
    b"\x1b[5Z",
    b"\x1b7",
    b"\x1b8",
    b"\n\n\n",
    b"\x1b[3S",
    b"qxlkmjtuvwn",
];

fn poke_view(t: &mut Terminal<'_>, rng: &mut XorShift) {
    match rng.next() % 6 {
        0 => t.scroll_view_back((rng.next() % 40) as usize),
        1 => t.scroll_view_forward((rng.next() % 40) as usize),
        2 => t.snap_to_live(),
        _ => {}
    }
}

/// Com tela alternativa e histórico: trocas de tela, rolagem da visão e tabulação
/// misturadas a bytes soltos e a CSI bem formado.
#[test]
fn random_screen_switches_scrolling_and_tabs_keep_the_invariants() {
    for (seed, (cols, rows, lines)) in [
        (10, (1, 1, 2)),
        (11, (5, 3, 4)),
        (12, (80, 24, 50)),
        (13, (7, 256, 3)),
        (14, (3, 8, 1)),
    ] {
        let mut rng = XorShift(0xFEED_FACE_0BAD_F00D ^ seed);
        full!(t, cols, rows, lines);
        let limit = rows.max(cols) as u64 + 3;
        for step in 0..30_000 {
            match rng.next() % 4 {
                0 => t.write(
                    SCREEN_SEQUENCES[usize::try_from(rng.next()).unwrap() % SCREEN_SEQUENCES.len()],
                ),
                1 => t.write(&well_formed_csi(&mut rng, limit)),
                _ => {
                    let len = 1 + (rng.next() % 24) as usize;
                    let chunk: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
                    t.write(&chunk);
                }
            }
            poke_view(&mut t, &mut rng);
            check_invariants(&t);
            assert!(t.take_dirty().all(|row| row < rows), "step {step}");
            reply(&mut t);
        }
    }
}

#[test]
fn random_bytes_end_with_the_same_state_whatever_the_chunking() {
    let mut rng = XorShift(0xDEAD_BEEF_CAFE_F00D);
    let bytes: Vec<u8> = (0..60_000).map(|_| rng.byte()).collect();
    let block = run_chunked(&bytes, &[bytes.len()]);
    assert_eq!(block, run_chunked(&bytes, &[1]));
    assert_eq!(block, run_chunked(&bytes, &[13, 2, 5, 31, 1, 1, 100]));
}

type FullSnapshot = (Vec<Cell>, Cursor, bool, usize, Vec<Cell>);

fn run_chunked_full(bytes: &[u8], sizes: &[usize]) -> FullSnapshot {
    full!(t, 17, 9, 12);
    let mut at = 0;
    let mut i = 0;
    while at < bytes.len() {
        let end = (at + sizes[i % sizes.len()]).min(bytes.len());
        t.write(&bytes[at..end]);
        at = end;
        i += 1;
    }
    let (live, cursor) = snapshot(&t);
    let (alt, history) = (t.is_alternate(), t.history_len());
    t.scroll_view_back(usize::MAX);
    let (oldest, _) = snapshot(&t);
    (live, cursor, alt, history, oldest)
}

#[test]
fn random_bytes_with_screens_end_with_the_same_state_whatever_the_chunking() {
    let mut rng = XorShift(0xABAD_1DEA_5EED_C0DE);
    let mut bytes = Vec::new();
    for _ in 0..4_000 {
        if rng.next().is_multiple_of(8) {
            bytes.extend_from_slice(
                SCREEN_SEQUENCES[usize::try_from(rng.next()).unwrap() % SCREEN_SEQUENCES.len()],
            );
        }
        bytes.extend((0..8).map(|_| rng.byte()));
    }
    let block = run_chunked_full(&bytes, &[bytes.len()]);
    assert_eq!(block, run_chunked_full(&bytes, &[1]));
    assert_eq!(block, run_chunked_full(&bytes, &[13, 2, 5, 31, 1, 1, 100]));
}

#[test]
fn the_screens_script_exercises_both_screens_and_the_history() {
    let mut script = Vec::new();
    for i in 0..30 {
        script.extend_from_slice(format!("line {i}\r\n").as_bytes());
    }
    script.extend_from_slice(b"\x1b[?1049h\x1b(0lqqk\x1b(B\x1b[3;3Halt\x1b[?1049l\x1b[3J");
    script.extend_from_slice(b"\x1b[3g\x1bH\t\x1b[Zend");
    let block = run_chunked_full(&script, &[script.len()]);
    assert_eq!(block, run_chunked_full(&script, &[1]));
    assert!(!block.2);
    assert_eq!(block.3, 0);
}
