//! Tela alternativa, histórico, desenho de caixas e paradas de tabulação.

use super::*;

const ALT_ON: &[u8] = b"\x1b[?1049h";
const ALT_OFF: &[u8] = b"\x1b[?1049l";

fn all_dirty(t: &mut Terminal<'_>) -> bool {
    dirty(t) == (0..t.rows()).collect::<Vec<_>>()
}

fn scrolled(t: &mut Terminal<'_>, lines: usize) -> Vec<String> {
    t.scroll_view_back(lines);
    screen(t)
}

// ---- tela alternativa ----

#[test]
fn mode_1049_saves_the_cursor_clears_the_alternate_screen_and_restores_it_on_exit() {
    full!(t, 8, 4, 0);
    t.write(b"main\x1b[2;3H\x1b[1;32m");
    t.write(ALT_ON);
    assert!(t.is_alternate());
    assert!(screen(&t).iter().all(String::is_empty));
    assert_eq!(pos(&t), (1, 2));
    t.write(b"\x1b[4;1Halt");
    t.write(ALT_OFF);
    assert!(!t.is_alternate());
    assert_eq!(screen(&t), ["main", "", "", ""]);
    assert_eq!(pos(&t), (1, 2));
    t.write(b"X");
    assert!(!at(&t, 1, 2).flags.contains(Flags::DEFAULT_FG));
}

#[test]
fn mode_1049_clears_the_alternate_screen_on_every_entry() {
    full!(t, 8, 3, 0);
    t.write(ALT_ON);
    t.write(b"first");
    t.write(ALT_OFF);
    t.write(ALT_ON);
    assert_eq!(screen(&t), ["", "", ""]);
    t.write(b"again");
    t.write(ALT_ON);
    assert!(t.is_alternate());
    assert_eq!(screen(&t), ["", "", ""]);
}

#[test]
fn mode_1049_leaves_the_main_screen_untouched_after_a_round_trip_with_scrolling() {
    full!(t, 6, 3, 8);
    numbered(&mut t, 7);
    let before = snapshot(&t);
    t.write(ALT_ON);
    numbered(&mut t, 9);
    t.write(b"\x1b[2J\x1b[1;1Hzz");
    t.write(ALT_OFF);
    t.write(b"\x1b[3;2H");
    assert_eq!(snapshot(&t).0, before.0);
    assert_eq!(t.history_len(), 4);
}

#[test]
fn mode_47_switches_without_clearing_or_saving_the_cursor() {
    full!(t, 8, 3, 0);
    t.write(b"main\x1b[2;3H\x1b[?47h");
    assert!(t.is_alternate());
    assert!(screen(&t).iter().all(String::is_empty));
    assert_eq!(pos(&t), (1, 2));
    t.write(b"\x1b[3;5HX\x1b[?47l");
    assert_eq!(screen(&t), ["main", "", ""]);
    assert_eq!(pos(&t), (2, 5));
    t.write(b"\x1b[?47h");
    assert_eq!(screen(&t), ["", "", "    X"]);
}

#[test]
fn mode_47_when_already_on_that_screen_does_nothing() {
    full!(t, 8, 3, 0);
    t.write(b"\x1b[?47l");
    assert!(!t.is_alternate());
    t.write(b"a\x1b[?47h\x1b[?47hb");
    assert!(t.is_alternate());
    assert_eq!(screen(&t), [" b", "", ""]);
    t.write(b"\x1b[?47l");
    assert_eq!(screen(&t), ["a", "", ""]);
}

#[test]
fn mode_1047_enters_without_clearing_and_clears_when_leaving() {
    full!(t, 8, 3, 0);
    t.write(b"main\x1b[?1047h");
    assert_eq!(screen(&t), ["", "", ""]);
    t.write(b"alt\x1b[?1047l");
    assert_eq!(screen(&t), ["main", "", ""]);
    t.write(b"\x1b[?1047h");
    assert_eq!(screen(&t), ["", "", ""]);
}

#[test]
fn mode_1047_reset_on_the_main_screen_clears_nothing() {
    full!(t, 8, 3, 0);
    t.write(b"main\x1b[?1047l");
    assert_eq!(screen(&t), ["main", "", ""]);
}

#[test]
fn mode_1047_keeps_what_mode_47_left_behind() {
    full!(t, 8, 3, 0);
    t.write(b"\x1b[?47hkeep\x1b[?47l\x1b[?1047h");
    assert_eq!(screen(&t), ["keep", "", ""]);
}

#[test]
fn mode_1048_only_saves_and_restores_the_cursor() {
    full!(t, 8, 3, 0);
    t.write(b"\x1b[2;4H\x1b[?1048h\x1b[3;1Hxy\x1b[?1048l");
    assert_eq!(pos(&t), (1, 3));
    assert_eq!(screen(&t), ["", "", "xy"]);
    assert!(!t.is_alternate());
}

#[test]
fn decsc_slots_are_one_per_screen() {
    full!(t, 8, 5, 0);
    t.write(b"\x1b[2;2H\x1b7\x1b[?47h\x1b[4;4H\x1b7\x1b[1;1H\x1b8");
    assert_eq!(pos(&t), (3, 3));
    t.write(b"\x1b[?47l\x1b[5;5H\x1b8");
    assert_eq!(pos(&t), (1, 1));
}

#[test]
fn mode_1049_round_trip_keeps_the_pending_wrap_and_the_pen() {
    full!(t, 3, 3, 0);
    t.write(b"\x1b[31mabc");
    t.write(ALT_ON);
    t.write(b"\x1b[0mxyz");
    t.write(ALT_OFF);
    t.write(b"d");
    assert_eq!(screen(&t), ["abc", "d", ""]);
    assert_eq!(at(&t, 1, 0).fg, 1);
}

#[test]
fn the_scroll_region_and_modes_are_shared_by_both_screens() {
    full!(t, 8, 6, 0);
    t.write(b"\x1b[2;4r\x1b[?25l");
    t.write(ALT_ON);
    assert!(!t.cursor().visible);
    t.write(b"\x1b[4;1H\n");
    assert_eq!(pos(&t), (3, 0));
    assert_eq!((t.scroll_top, t.scroll_bottom), (1, 3));
}

#[test]
fn switching_screens_marks_every_row_dirty() {
    full!(t, 8, 4, 0);
    dirty(&mut t);
    t.write(ALT_ON);
    assert!(all_dirty(&mut t));
    t.write(ALT_OFF);
    assert!(all_dirty(&mut t));
    t.write(b"\x1b[?47h");
    assert!(all_dirty(&mut t));
}

#[test]
fn the_main_screen_ring_survives_a_visit_to_the_alternate_screen() {
    full!(t, 4, 3, 0);
    numbered(&mut t, 8);
    let before = screen(&t);
    t.write(ALT_ON);
    numbered(&mut t, 5);
    t.write(ALT_OFF);
    assert_eq!(screen(&t), before);
    t.write(b"\r\nx");
    assert_eq!(screen(&t), ["7", "8", "x"]);
}

#[test]
fn without_alternate_memory_the_modes_change_nothing_but_the_cursor_slot() {
    term!(t, 8, 3);
    t.write(b"main\x1b[2;3H\x1b[?1049h");
    assert!(!t.is_alternate());
    t.write(b"\x1b[3;1Hx\x1b[?1049l");
    assert_eq!(screen(&t), ["main", "", "x"]);
    assert_eq!(pos(&t), (1, 2));
    t.write(b"\x1b[?47h\x1b[?1047h\x1b[?1047l");
    assert_eq!(screen(&t), ["main", "", "x"]);
}

#[test]
fn a_short_alternate_slice_is_refused() {
    let mut main = vec![Cell::BLANK; 24];
    let mut alt = vec![Cell::BLANK; 23];
    let mut t = Terminal::new(&mut main, 8, 3).unwrap();
    assert!(!t.set_alternate(&mut alt));
    t.write(b"\x1b[?47h");
    assert!(!t.is_alternate());
}

#[test]
fn the_alternate_slice_starts_blank_whatever_it_held() {
    let mut main = vec![Cell::BLANK; 24];
    let mut alt = vec![
        Cell {
            ch: 'z',
            ..Cell::BLANK
        };
        24
    ];
    let mut t = Terminal::new(&mut main, 8, 3).unwrap();
    assert!(t.set_alternate(&mut alt));
    t.write(b"\x1b[?47h");
    assert!(screen(&t).iter().all(String::is_empty));
}

#[test]
fn ris_leaves_the_alternate_screen_and_clears_both() {
    full!(t, 8, 3, 4);
    t.write(b"main\x1b[?47hdata\x1bc");
    assert!(!t.is_alternate());
    assert!(screen(&t).iter().all(String::is_empty));
    t.write(b"\x1b[?47h");
    assert!(screen(&t).iter().all(String::is_empty));
}

// ---- histórico ----

#[test]
fn lines_scrolled_off_the_top_go_to_the_history() {
    full!(t, 4, 3, 10);
    numbered(&mut t, 6);
    assert_eq!(screen(&t), ["4", "5", "6"]);
    assert_eq!(t.history_len(), 3);
    assert_eq!(scrolled(&mut t, 2), ["2", "3", "4"]);
    assert_eq!(scrolled(&mut t, 1), ["1", "2", "3"]);
}

#[test]
fn the_history_capacity_counts_whole_lines() {
    full!(t, 4, 3, 7);
    assert_eq!(t.history_capacity(), 7);
    term!(u, 4, 3);
    assert_eq!(u.history_capacity(), 0);
}

#[test]
fn the_view_stops_at_the_oldest_line() {
    full!(t, 4, 3, 10);
    numbered(&mut t, 5);
    t.scroll_view_back(100);
    assert_eq!(t.view_offset(), 2);
    assert_eq!(screen(&t), ["1", "2", "3"]);
    t.scroll_view_back(usize::MAX);
    assert_eq!(t.view_offset(), 2);
}

#[test]
fn scrolling_the_view_back_and_forth_returns_to_the_live_screen() {
    full!(t, 4, 3, 10);
    numbered(&mut t, 8);
    let live = screen(&t);
    t.scroll_view_back(3);
    assert_eq!(screen(&t), ["3", "4", "5"]);
    t.scroll_view_forward(1);
    assert_eq!(screen(&t), ["4", "5", "6"]);
    t.scroll_view_forward(10);
    assert_eq!(t.view_offset(), 0);
    assert_eq!(screen(&t), live);
}

#[test]
fn a_full_history_ring_drops_the_oldest_lines() {
    full!(t, 4, 2, 3);
    numbered(&mut t, 9);
    assert_eq!(t.history_len(), 3);
    assert_eq!(scrolled(&mut t, 99), ["5", "6"]);
    assert_eq!(screen(&t), ["5", "6"]);
    t.scroll_view_forward(1);
    assert_eq!(screen(&t), ["6", "7"]);
    t.write(b"\r\n10");
    assert_eq!(scrolled(&mut t, 99), ["6", "7"]);
}

#[test]
fn the_history_wraps_around_its_ring_many_times() {
    full!(t, 5, 3, 4);
    for i in 1..=100 {
        t.write(format!("{i}\r\n").as_bytes());
    }
    assert_eq!(t.history_len(), 4);
    assert_eq!(screen(&t), ["99", "100", ""]);
    t.scroll_view_back(1);
    assert_eq!(screen(&t), ["98", "99", "100"]);
    t.scroll_view_back(100);
    assert_eq!(screen(&t), ["95", "96", "97"]);
}

#[test]
fn history_lines_keep_their_colors_and_attributes() {
    full!(t, 4, 2, 4);
    t.write(b"\x1b[1;31mred\x1b[0m\r\n\r\n\r\n");
    t.scroll_view_back(5);
    let cell = at(&t, 0, 0);
    assert_eq!(cell.ch, 'r');
    assert_eq!(cell.fg, 1);
    assert!(cell.flags.contains(Flags::BOLD));
}

#[test]
fn output_snaps_the_view_back_to_the_live_screen() {
    full!(t, 4, 3, 10);
    numbered(&mut t, 6);
    t.scroll_view_back(2);
    t.write(b"");
    assert_eq!(t.view_offset(), 2);
    t.write(b"x");
    assert_eq!(t.view_offset(), 0);
    assert_eq!(line(&t, 2), "6x");
}

#[test]
fn snap_to_live_is_idempotent() {
    full!(t, 4, 3, 10);
    numbered(&mut t, 6);
    t.scroll_view_back(1);
    t.snap_to_live();
    t.snap_to_live();
    assert_eq!(t.view_offset(), 0);
}

#[test]
fn the_cursor_is_hidden_while_the_view_is_scrolled() {
    full!(t, 4, 3, 10);
    numbered(&mut t, 6);
    assert!(t.cursor().visible);
    t.scroll_view_back(1);
    assert!(!t.cursor().visible);
    t.scroll_view_forward(1);
    assert!(t.cursor().visible);
}

#[test]
fn moving_the_view_marks_every_row_dirty_and_a_clamped_move_marks_none() {
    full!(t, 4, 3, 10);
    numbered(&mut t, 6);
    dirty(&mut t);
    t.scroll_view_back(1);
    assert!(all_dirty(&mut t));
    t.scroll_view_forward(1);
    assert!(all_dirty(&mut t));
    t.scroll_view_forward(1);
    assert_eq!(dirty(&mut t), Vec::<usize>::new());
    t.scroll_view_back(100);
    dirty(&mut t);
    t.scroll_view_back(1);
    assert_eq!(dirty(&mut t), Vec::<usize>::new());
}

#[test]
fn ed_3_clears_the_history_and_leaves_screen_cursor_and_wrap_alone() {
    full!(t, 3, 3, 10);
    numbered(&mut t, 5);
    t.write(b"\r\nabc");
    t.write(b"\x1b[3J");
    assert_eq!(t.history_len(), 0);
    assert_eq!(screen(&t), ["4", "5", "abc"]);
    assert_eq!(pos(&t), (2, 2));
    t.write(b"d");
    assert_eq!(line(&t, 2), "d");
}

#[test]
fn ed_3_while_scrolled_returns_to_the_live_screen() {
    full!(t, 3, 3, 10);
    numbered(&mut t, 6);
    t.scroll_view_back(2);
    t.write(b"\x1b[3J");
    assert_eq!(t.view_offset(), 0);
    assert_eq!(t.history_len(), 0);
    t.scroll_view_back(1);
    assert_eq!(t.view_offset(), 0);
}

#[test]
fn other_erases_do_not_touch_the_history() {
    full!(t, 4, 3, 10);
    numbered(&mut t, 6);
    t.write(b"\x1b[2J\x1b[H\x1b[J\x1b[1J\x1b[2K\x1b[4J\x1b[9K");
    assert_eq!(t.history_len(), 3);
    assert_eq!(scrolled(&mut t, 3), ["1", "2", "3"]);
}

#[test]
fn an_unknown_ed_or_el_does_not_cancel_the_pending_wrap() {
    term!(t, 3, 3);
    t.write(b"abc\x1b[4J\x1b[3KX");
    assert_eq!(screen(&t), ["abc", "X", ""]);
}

#[test]
fn su_and_a_region_from_the_top_feed_the_history_but_other_regions_do_not() {
    full!(t, 4, 5, 10);
    t.write(b"a\r\nb\r\nc\r\nd\r\ne\x1b[2S");
    assert_eq!(t.history_len(), 2);
    assert_eq!(scrolled(&mut t, 2), ["a", "b", "c", "d", "e"]);
    t.snap_to_live();
    t.write(b"\x1b[1;3r\x1b[3;1H\n\n");
    assert_eq!(t.history_len(), 4);
    t.write(b"\x1b[2;4r\x1b[4;1H\n\n\n");
    assert_eq!(t.history_len(), 4);
}

#[test]
fn dl_at_the_top_row_feeds_the_history() {
    full!(t, 4, 3, 10);
    t.write(b"a\r\nb\r\nc\x1b[1;1H\x1b[M");
    assert_eq!(t.history_len(), 1);
    assert_eq!(scrolled(&mut t, 1), ["a", "b", "c"]);
}

#[test]
fn the_alternate_screen_never_writes_the_history() {
    full!(t, 4, 3, 10);
    numbered(&mut t, 5);
    assert_eq!(t.history_len(), 2);
    t.write(ALT_ON);
    numbered(&mut t, 20);
    t.write(b"\x1b[2S\x1b[M");
    assert_eq!(t.history_len(), 2);
    t.scroll_view_back(5);
    assert_eq!(t.view_offset(), 0);
    t.write(ALT_OFF);
    assert_eq!(t.history_len(), 2);
    assert_eq!(scrolled(&mut t, 2), ["1", "2", "3"]);
}

#[test]
fn entering_the_alternate_screen_returns_the_view_to_the_live_screen() {
    full!(t, 4, 3, 10);
    numbered(&mut t, 6);
    t.scroll_view_back(2);
    t.write(ALT_ON);
    assert_eq!(t.view_offset(), 0);
    t.write(ALT_OFF);
    assert_eq!(screen(&t), ["4", "5", "6"]);
}

#[test]
fn without_history_memory_scrolling_back_does_nothing() {
    full!(t, 4, 3, 0);
    numbered(&mut t, 9);
    assert_eq!(t.history_len(), 0);
    t.scroll_view_back(3);
    assert_eq!(t.view_offset(), 0);
    assert_eq!(screen(&t), ["7", "8", "9"]);
    term!(u, 4, 3);
    numbered(&mut u, 9);
    u.scroll_view_back(3);
    assert_eq!(u.view_offset(), 0);
}

#[test]
fn history_slice_shorter_than_a_line_disables_it() {
    let mut main = vec![Cell::BLANK; 12];
    let mut hist = vec![Cell::BLANK; 3];
    let mut t = Terminal::new(&mut main, 4, 3).unwrap();
    t.set_history(&mut hist);
    numbered(&mut t, 9);
    assert_eq!(t.history_len(), 0);
}

#[test]
fn ris_clears_the_history_and_the_view() {
    full!(t, 4, 3, 10);
    numbered(&mut t, 9);
    t.scroll_view_back(2);
    t.write(b"\x1bc");
    assert_eq!(t.history_len(), 0);
    assert_eq!(t.view_offset(), 0);
}

#[test]
fn the_history_follows_the_logical_order_after_the_main_ring_rotates() {
    full!(t, 3, 4, 20);
    for i in 0..30 {
        t.write(format!("{i:02}\r\n").as_bytes());
    }
    t.scroll_view_back(20);
    let mut seen = Vec::new();
    for row in 0..4 {
        seen.push(line(&t, row));
    }
    assert_eq!(seen, ["07", "08", "09", "10"]);
}

#[test]
fn a_view_longer_than_the_history_never_indexes_out_of_range() {
    full!(t, 3, 5, 2);
    numbered(&mut t, 6);
    assert_eq!(t.history_len(), 1);
    t.scroll_view_back(1);
    assert_eq!(screen(&t), ["1", "2", "3", "4", "5"]);
    assert_eq!(t.row(5), &[] as &[Cell]);
}

// ---- DEC Special Graphics ----

#[test]
fn designating_graphics_to_g0_draws_boxes() {
    term!(t, 8, 4);
    t.write(b"\x1b(0lqqqk\r\nx   x\r\nmqqqj\x1b(B");
    assert_eq!(screen(&t), ["┌───┐", "│   │", "└───┘", ""]);
}

#[test]
fn every_box_letter_maps_to_its_glyph() {
    term!(t, 20, 2);
    t.write(b"\x1b(0qxlkmjtuvwn");
    assert_eq!(line(&t, 0), "─│┌┐└┘├┤┴┬┼");
}

#[test]
fn esc_paren_b_returns_to_ascii() {
    term!(t, 10, 2);
    t.write(b"\x1b(0q\x1b(Bq\x1b(0q");
    assert_eq!(line(&t, 0), "─q─");
}

#[test]
fn the_rest_of_the_set_uses_the_xterm_table() {
    term!(t, 10, 2);
    t.write(b"\x1b(0afg}~`");
    assert_eq!(line(&t, 0), "▒°±£·◆");
}

#[test]
fn characters_outside_the_graphics_range_pass_through() {
    term!(t, 20, 2);
    t.write(b"\x1b(0 A1z\x7f");
    assert_eq!(line(&t, 0), " A1≥");
    t.write("é€".as_bytes());
    assert_eq!(line(&t, 0), " A1≥é€");
}

#[test]
fn unknown_designations_fall_back_to_ascii() {
    term!(t, 10, 2);
    for fin in *b"AB12K" {
        t.write(b"\x1b(0");
        t.write(&[0x1B, b'(', fin]);
        t.write(b"q");
    }
    assert_eq!(line(&t, 0), "qqqqq");
}

#[test]
fn designating_prints_nothing_and_leaves_the_cursor() {
    term!(t, 10, 2);
    t.write(b"a\x1b(0\x1b)0\x1b(B");
    assert_eq!(pos(&t), (0, 1));
    assert_eq!(line(&t, 0), "a");
}

#[test]
fn shift_out_invokes_g1_and_shift_in_returns_to_g0() {
    term!(t, 10, 2);
    t.write(b"\x1b)0q\x0eq\x0fq\x0eq\x0f");
    assert_eq!(line(&t, 0), "q─q─");
}

#[test]
fn g1_without_shift_out_has_no_effect_and_g0_graphics_vanishes_under_so() {
    term!(t, 10, 2);
    t.write(b"\x1b)0q\x0e\x1b(0\x1b)Bq\x0fq");
    assert_eq!(line(&t, 0), "qq─");
}

#[test]
fn graphics_works_across_the_ascii_fast_path_and_wraps() {
    term!(t, 4, 3);
    t.write(b"\x1b(0qqqqqqqqqq");
    assert_eq!(screen(&t), ["────", "────", "──"]);
}

#[test]
fn graphics_survives_a_write_cut_at_every_byte() {
    let text = b"\x1b(0lqk\x1b(Bq\x0e";
    for split in 0..=text.len() {
        term!(t, 10, 2);
        t.write(&text[..split]);
        t.write(&text[split..]);
        assert_eq!(line(&t, 0), "┌─┐q", "split at {split}");
    }
}

#[test]
fn repeat_repeats_the_mapped_character() {
    term!(t, 10, 2);
    t.write(b"\x1b(0q\x1b[3b");
    assert_eq!(line(&t, 0), "────");
}

#[test]
fn decsc_saves_and_decrc_restores_the_character_sets_and_the_shift() {
    term!(t, 10, 2);
    t.write(b"\x1b(0\x1b)0\x0e\x1b7\x1b(B\x0f\x1b8q");
    assert_eq!(line(&t, 0), "─");
    t.write(b"\x1b(B\x0f\x1b[2;1H\x1b[uq");
    assert_eq!(line(&t, 0), "─");
}

#[test]
fn decrc_without_a_save_restores_ascii() {
    term!(t, 10, 2);
    t.write(b"\x1b(0\x1b8q");
    assert_eq!(line(&t, 0), "q");
}

#[test]
fn ris_resets_the_character_sets() {
    term!(t, 10, 2);
    t.write(b"\x1b(0\x1b)0\x0e\x1bcq");
    assert_eq!(line(&t, 0), "q");
    t.write(b"\x0eq");
    assert_eq!(line(&t, 0), "qq");
}

#[test]
fn graphics_keeps_the_pen_colors() {
    term!(t, 10, 2);
    t.write(b"\x1b[31m\x1b(0q");
    assert_eq!(at(&t, 0, 0).ch, '─');
    assert_eq!(at(&t, 0, 0).fg, 1);
}

#[test]
fn the_character_set_is_the_same_on_both_screens() {
    full!(t, 10, 2, 0);
    t.write(b"\x1b(0\x1b[?1049hq\x1b[?1049lq");
    assert_eq!(line(&t, 0), "─");
}

// ---- paradas de tabulação ----

#[test]
fn hts_sets_a_stop_at_the_cursor_column() {
    term!(t, 40, 2);
    t.write(b"\x1b[1;4H\x1bH\r\t");
    assert_eq!(pos(&t), (0, 3));
    t.write(b"\t");
    assert_eq!(pos(&t), (0, 8));
}

#[test]
fn tbc_zero_clears_only_the_stop_under_the_cursor() {
    term!(t, 40, 2);
    t.write(b"\x1b[1;9H\x1b[g\r\t");
    assert_eq!(pos(&t), (0, 16));
    t.write(b"\x1b[1;17H\x1b[0g\r\t");
    assert_eq!(pos(&t), (0, 24));
}

#[test]
fn tbc_three_clears_every_stop() {
    term!(t, 40, 2);
    t.write(b"\x1b[3g\t");
    assert_eq!(pos(&t), (0, 39));
    t.write(b"\x1b[1;1H\x1b[5G\x1bH\x1b[1G\t");
    assert_eq!(pos(&t), (0, 4));
}

#[test]
fn tbc_with_other_parameters_is_ignored() {
    term!(t, 40, 2);
    t.write(b"\x1b[1;9H\x1b[1g\x1b[2g\x1b[4g\x1b[?3g\r\t");
    assert_eq!(pos(&t), (0, 8));
}

#[test]
fn cht_moves_forward_by_stops_and_defaults_to_one() {
    term!(t, 40, 2);
    t.write(b"\x1b[I");
    assert_eq!(pos(&t), (0, 8));
    t.write(b"\x1b[2I");
    assert_eq!(pos(&t), (0, 24));
    t.write(b"\x1b[0I");
    assert_eq!(pos(&t), (0, 32));
    t.write(b"\x1b[99I");
    assert_eq!(pos(&t), (0, 39));
}

#[test]
fn cbt_moves_backward_by_stops_and_stops_at_column_zero() {
    term!(t, 40, 2);
    t.write(b"\x1b[1;20H\x1b[Z");
    assert_eq!(pos(&t), (0, 16));
    t.write(b"\x1b[2Z");
    assert_eq!(pos(&t), (0, 0));
    t.write(b"\x1b[Z\x1b[1;4H\x1b[Z");
    assert_eq!(pos(&t), (0, 0));
    t.write(b"\x1b[1;30H\x1b[0Z\x1b[999Z");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn cht_and_cbt_follow_custom_stops() {
    term!(t, 40, 2);
    t.write(b"\x1b[3g\x1b[1;6H\x1bH\x1b[1;12H\x1bH\x1b[1G");
    t.write(b"\x1b[I");
    assert_eq!(pos(&t), (0, 5));
    t.write(b"\x1b[I");
    assert_eq!(pos(&t), (0, 11));
    t.write(b"\x1b[Z");
    assert_eq!(pos(&t), (0, 5));
}

#[test]
fn a_tab_with_a_pending_wrap_stays_pending() {
    term!(t, 5, 3);
    t.write(b"abcde\tX");
    assert_eq!(screen(&t), ["abcde", "X", ""]);
}

#[test]
fn cht_and_cbt_keep_the_pending_wrap_but_other_motion_clears_it() {
    term!(t, 5, 3);
    t.write(b"abcde\x1b[IX");
    assert_eq!(screen(&t), ["abcde", "X", ""]);
    term!(u, 5, 3);
    u.write(b"abcde\x1b[ZX");
    assert_eq!(screen(&u), ["abcde", "X", ""]);
}

#[test]
fn ris_restores_the_default_stops() {
    term!(t, 40, 2);
    t.write(b"\x1b[3g\x1b[1;5H\x1bH\x1bc\t");
    assert_eq!(pos(&t), (0, 8));
}

#[test]
fn the_stops_are_the_same_on_both_screens() {
    full!(t, 40, 3, 0);
    t.write(b"\x1b[3g\x1b[1;5H\x1bH\x1b[?47h\x1b[1G\t");
    assert_eq!(pos(&t), (0, 4));
}

#[test]
fn a_grid_wider_than_the_stop_bitmap_is_refused() {
    let mut cells = vec![Cell::BLANK; (MAX_COLS + 1) * 2];
    assert!(Terminal::new(&mut cells, MAX_COLS + 1, 2).is_none());
    assert!(Terminal::new(&mut cells, MAX_COLS, 2).is_some());
}

#[test]
fn tabs_past_the_last_default_stop_reach_the_last_column() {
    term!(t, 12, 2);
    t.write(b"\t\t\t");
    assert_eq!(pos(&t), (0, 11));
}

// ---- conferência contra o xterm: quebra pendente, DECSTBM e DECOM ----

#[test]
fn toggling_autowrap_does_not_cancel_a_pending_wrap() {
    term!(t, 3, 2);
    t.write(b"abc\x1b[?7l\x1b[?7hX");
    assert_eq!(screen(&t), ["abc", "X"]);
}

#[test]
fn erase_in_line_and_character_cancel_the_pending_wrap() {
    for erase in [
        &b"\x1b[K"[..],
        b"\x1b[1K",
        b"\x1b[2K",
        b"\x1b[X",
        b"\x1b[J",
        b"\x1b[1J",
    ] {
        term!(t, 3, 2);
        t.write(b"abc");
        t.write(erase);
        t.write(b"Z");
        assert_eq!(pos(&t), (0, 2), "{erase:?}");
        assert_eq!(at(&t, 0, 2).ch, 'Z', "{erase:?}");
    }
}

#[test]
fn insert_and_delete_character_cancel_the_pending_wrap() {
    for edit in [&b"\x1b[@"[..], b"\x1b[P"] {
        term!(t, 3, 2);
        t.write(b"abc");
        t.write(edit);
        t.write(b"Z");
        assert_eq!(pos(&t), (0, 2), "{edit:?}");
    }
}

#[test]
fn decstbm_homes_the_cursor_to_the_region_top_in_origin_mode() {
    term!(t, 10, 8);
    t.write(b"\x1b[?6h\x1b[3;6r");
    assert_eq!(pos(&t), (2, 0));
    t.write(b"\x1b[?6l\x1b[4;5r");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn decstbm_with_a_zero_bottom_means_the_last_row_and_a_bad_top_is_ignored() {
    term!(t, 10, 6);
    t.write(b"\x1b[2;0r");
    assert_eq!((t.scroll_top, t.scroll_bottom), (1, 5));
    t.write(b"\x1b[0;4r");
    assert_eq!((t.scroll_top, t.scroll_bottom), (0, 3));
    t.write(b"\x1b[7;9r");
    assert_eq!((t.scroll_top, t.scroll_bottom), (0, 3));
    t.write(b"\x1b[3;2r\x1b[4;4r");
    assert_eq!((t.scroll_top, t.scroll_bottom), (0, 3));
}

#[test]
fn decom_set_and_reset_both_home_the_cursor() {
    term!(t, 10, 8);
    t.write(b"\x1b[3;6r\x1b[5;5H\x1b[?6h");
    assert_eq!(pos(&t), (2, 0));
    t.write(b"\x1b[4;4H\x1b[?6l");
    assert_eq!(pos(&t), (0, 0));
}

#[test]
fn decrc_in_origin_mode_keeps_the_cursor_inside_the_region() {
    term!(t, 10, 8);
    t.write(b"\x1b[?6h\x1b[2;5r\x1b[3;3H\x1b7");
    assert_eq!(pos(&t), (3, 2));
    t.write(b"\x1b[5;8r\x1b8");
    assert_eq!(pos(&t), (4, 2));
    t.write(b"\x1b[1;2r\x1b8");
    assert_eq!(pos(&t), (1, 2));
}

// ---- memória entregue depois do `new` ----

#[test]
fn the_alternate_slice_is_refused_while_the_alternate_screen_is_active() {
    full!(t, 4, 2, 0);
    t.write(b"main\x1b[?47h");
    let mut other = vec![Cell::BLANK; 8];
    assert!(!t.set_alternate(&mut other));
    t.write(b"alt\x1b[?47l");
    assert_eq!(screen(&t), ["main", ""]);
}

#[test]
fn a_new_alternate_slice_replaces_the_old_one_from_the_main_screen() {
    full!(t, 4, 2, 0);
    t.write(b"\x1b[?47hold\x1b[?47l");
    let mut fresh = vec![Cell::BLANK; 8];
    assert!(t.set_alternate(&mut fresh));
    t.write(b"\x1b[?47h");
    assert_eq!(screen(&t), ["", ""]);
}

#[test]
fn history_attached_late_starts_empty_and_keeps_the_screen() {
    term!(t, 4, 3);
    numbered(&mut t, 6);
    assert_eq!(t.history_len(), 0);
    let mut hist = vec![Cell::BLANK; 4 * 5];
    t.set_history(&mut hist);
    assert_eq!((t.history_len(), t.history_capacity()), (0, 5));
    assert_eq!(screen(&t), ["4", "5", "6"]);
    t.write(b"\r\n7\r\n8");
    assert_eq!(t.history_len(), 2);
    assert_eq!(scrolled(&mut t, 2), ["4", "5", "6"]);
}

#[test]
fn replacing_the_history_returns_the_view_to_the_live_screen() {
    full!(t, 4, 3, 10);
    numbered(&mut t, 6);
    t.scroll_view_back(2);
    let mut hist = vec![Cell::BLANK; 8];
    t.set_history(&mut hist);
    assert_eq!((t.view_offset(), t.history_len()), (0, 0));
    assert_eq!(screen(&t), ["4", "5", "6"]);
}

// ---- teclas que rolam e teclas que voltam à tela viva ----

fn press(t: &mut Terminal<'_>, key: Key) -> Vec<u8> {
    let mut out = [0u8; 8];
    let n = t.press(key, &mut out);
    out[..n].to_vec()
}

#[test]
fn scroll_keys_move_the_view_by_half_a_screen_and_send_nothing() {
    full!(t, 4, 6, 20);
    numbered(&mut t, 20);
    assert_eq!(press(&mut t, Key::ScrollBack), Vec::<u8>::new());
    assert_eq!(t.view_offset(), 3);
    press(&mut t, Key::ScrollBack);
    assert_eq!(t.view_offset(), 6);
    press(&mut t, Key::ScrollForward);
    assert_eq!(t.view_offset(), 3);
    assert_eq!(press(&mut t, Key::ScrollForward), Vec::<u8>::new());
    assert_eq!(t.view_offset(), 0);
}

#[test]
fn a_page_is_at_least_one_line() {
    full!(t, 4, 1, 5);
    numbered(&mut t, 4);
    press(&mut t, Key::ScrollBack);
    assert_eq!(t.view_offset(), 1);
}

#[test]
fn any_other_key_returns_to_the_live_screen_and_is_encoded() {
    full!(t, 4, 4, 20);
    numbered(&mut t, 12);
    for (key, bytes) in [
        (Key::Char('a'), &b"a"[..]),
        (Key::Enter, b"\r"),
        (Key::PageUp, b"\x1b[5~"),
        (Key::Up, b"\x1b[A"),
    ] {
        t.scroll_view_back(3);
        assert_eq!(press(&mut t, key), bytes, "{key:?}");
        assert_eq!(t.view_offset(), 0, "{key:?}");
    }
}

#[test]
fn keys_without_bytes_still_return_to_the_live_screen() {
    full!(t, 4, 4, 20);
    numbered(&mut t, 12);
    t.scroll_view_back(3);
    assert_eq!(press(&mut t, Key::F(13)), Vec::<u8>::new());
    assert_eq!(t.view_offset(), 0);
}

#[test]
fn scroll_keys_do_nothing_on_the_alternate_screen_and_pass_no_bytes() {
    full!(t, 4, 4, 20);
    numbered(&mut t, 12);
    t.write(ALT_ON);
    assert_eq!(press(&mut t, Key::ScrollBack), Vec::<u8>::new());
    assert_eq!(t.view_offset(), 0);
}

#[test]
fn press_respects_the_application_cursor_mode() {
    full!(t, 4, 4, 0);
    t.write(b"\x1b[?1h");
    assert_eq!(press(&mut t, Key::Up), b"\x1bOA");
}

// ---- parâmetros enormes e fatias folgadas ----

#[test]
fn csi_with_huge_parameters_stays_inside_the_grid() {
    const FINALS: &[u8] = b"IZSTLMrHfGdAB@PXCDEFJK";
    const PARAMS: &[&str] = &["65535", "65534", "0", "1", "2", "3", "99999", "70000"];
    let mut rng = XorShift(0x5EED_0000_FFFF_0001);
    for (cols, rows, lines, slack) in [
        (1, 1, 0, 0),
        (3, 2, 1, 17),
        (80, 24, 5, 3),
        (7, 256, 0, 40),
        (512, 4, 2, 1),
    ] {
        let mut main = vec![Cell::BLANK; Terminal::cells_needed(cols, rows) + slack];
        let mut alt = vec![Cell::BLANK; Terminal::cells_needed(cols, rows) + slack];
        let mut hist = vec![Cell::BLANK; Terminal::history_cells_needed(cols, lines) + slack];
        let mut t = Terminal::new(&mut main, cols, rows).unwrap();
        assert!(t.set_alternate(&mut alt));
        t.set_history(&mut hist);
        for _ in 0..20_000 {
            let mut seq = b"\x1b[".to_vec();
            for index in 0..rng.next() % 3 {
                if index > 0 {
                    seq.push(b';');
                }
                let pick = usize::try_from(rng.next()).unwrap() % PARAMS.len();
                seq.extend_from_slice(PARAMS[pick].as_bytes());
            }
            seq.push(FINALS[usize::try_from(rng.next()).unwrap() % FINALS.len()]);
            if rng.next().is_multiple_of(4) {
                seq.extend_from_slice(b"text\r\n\n");
            }
            t.write(&seq);
            check_invariants(&t);
            t.scroll_view_back(usize::MAX);
            check_invariants(&t);
            t.snap_to_live();
            assert!(t.take_dirty().all(|row| row < rows));
        }
    }
}

#[test]
fn the_largest_parameters_do_the_obvious_thing() {
    full!(t, 5, 4, 3);
    numbered(&mut t, 3);
    t.write(b"\x1b[65535S");
    assert_eq!(screen(&t), ["", "", "", ""]);
    assert_eq!(t.history_len(), 3);
    t.write(b"\x1b[65535T\x1b[65535L\x1b[65535M");
    assert_eq!(screen(&t), ["", "", "", ""]);
    t.write(b"\x1b[65535;65535r");
    assert_eq!((t.scroll_top, t.scroll_bottom), (0, 3));
    t.write(b"\x1b[65535I");
    assert_eq!(pos(&t), (2, 4));
    t.write(b"\x1b[65535Z");
    assert_eq!(pos(&t), (2, 0));
}

#[test]
fn slices_larger_than_needed_are_used_only_up_to_the_grid() {
    let mut main = vec![Cell::BLANK; 12 + 9];
    let mut alt = vec![Cell::BLANK; 12 + 9];
    let mut hist = vec![Cell::BLANK; 4 * 2 + 3];
    let mut t = Terminal::new(&mut main, 4, 3).unwrap();
    assert!(t.set_alternate(&mut alt));
    t.set_history(&mut hist);
    assert_eq!(t.history_capacity(), 2);
    numbered(&mut t, 7);
    t.write(ALT_ON);
    numbered(&mut t, 5);
    t.write(ALT_OFF);
    assert_eq!(screen(&t), ["5", "6", "7"]);
    assert_eq!(scrolled(&mut t, 9), ["3", "4", "5"]);
}
