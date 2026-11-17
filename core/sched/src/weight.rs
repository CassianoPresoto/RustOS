//! Peso por nice e a conversão entre tempo real e tempo virtual.

/// Peso de nice 0: a unidade do tempo virtual.
pub const NICE_0_WEIGHT: u32 = 1024;
pub const MIN_NICE: i8 = -20;
pub const MAX_NICE: i8 = 19;

/// `sched_prio_to_weight` de `kernel/sched/core.c` do Linux: 1,25 por nível.
const PRIO_TO_WEIGHT: [u32; 40] = [
    88761, 71755, 56483, 46273, 36291, 29154, 23254, 18705, 14949, 11916, 9548, 7620, 6100, 4904,
    3906, 3121, 2501, 1991, 1586, 1277, 1024, 820, 655, 526, 423, 335, 272, 215, 172, 137, 110, 87,
    70, 56, 45, 36, 29, 23, 18, 15,
];

/// Peso do nice, com o nice preso a `MIN_NICE..=MAX_NICE`.
pub fn weight_of_nice(nice: i8) -> u32 {
    let index = usize::from((nice.clamp(MIN_NICE, MAX_NICE) - MIN_NICE).cast_unsigned());
    PRIO_TO_WEIGHT[index]
}

/// Tempo real que a tarefa consumiu, dividido pelo peso: `delta * 1024 / peso`.
pub fn real_to_virtual(delta_ns: u64, weight: u32) -> u64 {
    let weight = u64::from(weight.max(1));
    if delta_ns <= u64::MAX >> 10 {
        (delta_ns << 10) / weight
    } else {
        let wide = (u128::from(delta_ns) << 10) / u128::from(weight);
        u64::try_from(wide).unwrap_or(u64::MAX)
    }
}

/// Inverso de `real_to_virtual`, arredondado para cima.
pub fn virtual_to_real(delta_virtual: u64, weight: u32) -> u64 {
    let wide = (u128::from(delta_virtual) * u128::from(weight)).div_ceil(1024);
    u64::try_from(wide).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nice_zero_is_the_unit_and_each_level_is_about_25_percent() {
        assert_eq!(weight_of_nice(0), NICE_0_WEIGHT);
        assert_eq!(weight_of_nice(-20), 88761);
        assert_eq!(weight_of_nice(19), 15);
        for nice in MIN_NICE..MAX_NICE {
            let ratio = weight_of_nice(nice) * 100 / weight_of_nice(nice + 1);
            assert!((120..130).contains(&ratio), "nice {nice}: {ratio}");
        }
    }

    #[test]
    fn nice_out_of_range_is_clamped() {
        assert_eq!(weight_of_nice(-128), weight_of_nice(MIN_NICE));
        assert_eq!(weight_of_nice(127), weight_of_nice(MAX_NICE));
    }

    #[test]
    fn virtual_time_scales_inversely_with_weight() {
        assert_eq!(real_to_virtual(1_000, 1024), 1_000);
        assert_eq!(real_to_virtual(1_000, 2048), 500);
        assert_eq!(real_to_virtual(1_000, 512), 2_000);
        assert_eq!(virtual_to_real(500, 2048), 1_000);
    }

    #[test]
    fn huge_deltas_do_not_overflow() {
        assert_eq!(real_to_virtual(u64::MAX, 1024), u64::MAX);
        assert_eq!(real_to_virtual(u64::MAX, 15), u64::MAX);
        assert_eq!(virtual_to_real(u64::MAX, 88761), u64::MAX);
    }
}
