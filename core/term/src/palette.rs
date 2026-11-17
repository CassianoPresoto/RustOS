//! Paleta xterm de 256 cores e a redução de RGB para o índice mais próximo.

const BASE16: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0xCD, 0x00, 0x00],
    [0x00, 0xCD, 0x00],
    [0xCD, 0xCD, 0x00],
    [0x00, 0x00, 0xEE],
    [0xCD, 0x00, 0xCD],
    [0x00, 0xCD, 0xCD],
    [0xE5, 0xE5, 0xE5],
    [0x7F, 0x7F, 0x7F],
    [0xFF, 0x00, 0x00],
    [0x00, 0xFF, 0x00],
    [0xFF, 0xFF, 0x00],
    [0x5C, 0x5C, 0xFF],
    [0xFF, 0x00, 0xFF],
    [0x00, 0xFF, 0xFF],
    [0xFF, 0xFF, 0xFF],
];

const fn cube_level(step: u8) -> u8 {
    if step == 0 { 0 } else { 55 + 40 * step }
}

/// RGB de um índice da paleta xterm: 0–15 padrão, 16–231 cubo 6x6x6, 232–255 cinzas.
pub const fn xterm_rgb(index: u8) -> [u8; 3] {
    if index < 16 {
        BASE16[index as usize]
    } else if index < 232 {
        let i = index - 16;
        [cube_level(i / 36), cube_level(i / 6 % 6), cube_level(i % 6)]
    } else {
        let gray = 8 + 10 * (index - 232);
        [gray, gray, gray]
    }
}

fn cube_step(value: u8) -> u8 {
    if value < 48 {
        0
    } else if value < 115 {
        1
    } else {
        (value - 35) / 40
    }
}

fn distance(a: [u8; 3], b: [u8; 3]) -> i32 {
    let mut sum = 0;
    for (x, y) in a.into_iter().zip(b) {
        let d = i32::from(x) - i32::from(y);
        sum += d * d;
    }
    sum
}

/// Índice (16–255) mais próximo de um RGB, só com inteiros; empate fica com o cubo.
pub(crate) fn nearest_index(r: u8, g: u8, b: u8) -> u8 {
    let cube = 16 + 36 * cube_step(r) + 6 * cube_step(g) + cube_step(b);
    let avg = (u16::from(r) + u16::from(g) + u16::from(b)) / 3;
    let step = (avg.saturating_sub(3) / 10).min(23);
    let gray = 232 + u8::try_from(step).unwrap_or(23);
    let want = [r, g, b];
    if distance(want, xterm_rgb(gray)) < distance(want, xterm_rgb(cube)) {
        gray
    } else {
        cube
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_colors_match_the_xterm_defaults() {
        assert_eq!(xterm_rgb(0), [0, 0, 0]);
        assert_eq!(xterm_rgb(1), [0xCD, 0, 0]);
        assert_eq!(xterm_rgb(7), [0xE5, 0xE5, 0xE5]);
        assert_eq!(xterm_rgb(8), [0x7F, 0x7F, 0x7F]);
        assert_eq!(xterm_rgb(12), [0x5C, 0x5C, 0xFF]);
        assert_eq!(xterm_rgb(15), [0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn color_cube_uses_the_six_xterm_levels() {
        assert_eq!(xterm_rgb(16), [0, 0, 0]);
        assert_eq!(xterm_rgb(17), [0, 0, 95]);
        assert_eq!(xterm_rgb(21), [0, 0, 255]);
        assert_eq!(xterm_rgb(196), [255, 0, 0]);
        assert_eq!(xterm_rgb(46), [0, 255, 0]);
        assert_eq!(xterm_rgb(231), [255, 255, 255]);
        assert_eq!(xterm_rgb(16 + 36 * 2 + 6 * 3 + 4), [135, 175, 215]);
    }

    #[test]
    fn gray_ramp_runs_from_8_to_238() {
        assert_eq!(xterm_rgb(232), [8, 8, 8]);
        assert_eq!(xterm_rgb(244), [128, 128, 128]);
        assert_eq!(xterm_rgb(255), [238, 238, 238]);
    }

    #[test]
    fn every_index_is_defined() {
        for index in 0..=255u8 {
            let _ = xterm_rgb(index);
        }
    }

    #[test]
    fn rgb_reduction_hits_exact_cube_and_gray_entries() {
        assert_eq!(nearest_index(255, 0, 0), 196);
        assert_eq!(nearest_index(0, 0, 0), 16);
        assert_eq!(nearest_index(255, 255, 255), 231);
        assert_eq!(nearest_index(0, 95, 135), 16 + 6 + 2);
        assert_eq!(nearest_index(128, 128, 128), 244);
        assert_eq!(nearest_index(8, 8, 8), 232);
        assert_eq!(nearest_index(238, 238, 238), 255);
    }

    #[test]
    fn rgb_reduction_picks_the_closer_of_cube_and_gray() {
        assert_eq!(nearest_index(100, 100, 100), 241);
        assert_eq!(nearest_index(95, 95, 95), 59);
        assert_eq!(nearest_index(250, 10, 10), 196);
    }

    #[test]
    fn rgb_reduction_never_returns_a_base_color() {
        for r in (0..=255).step_by(15) {
            for g in (0..=255).step_by(15) {
                for b in (0..=255).step_by(15) {
                    assert!(nearest_index(r, g, b) >= 16);
                }
            }
        }
    }
}
