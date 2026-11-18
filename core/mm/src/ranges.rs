//! Política de corridas do mapa direto: onde cabe página de 2 MiB e onde sobra 4 KiB,
//! e como regiões vizinhas viram uma faixa só. Pura: quem mapeia é o kernel.

use rustos_hal::{LARGE_PAGE_SIZE, PAGE_SIZE};

/// Um mapeamento a fazer: `virt` e `phys` do começo, de 2 MiB (`large`) ou de 4 KiB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub virt: u64,
    pub phys: u64,
    pub large: bool,
}

impl Run {
    pub const fn size(&self) -> u64 {
        if self.large {
            LARGE_PAGE_SIZE
        } else {
            PAGE_SIZE
        }
    }
}

/// As páginas que cobrem `[phys, phys + len)` em `virt`: 2 MiB onde o endereço virtual,
/// o físico e o que resta alinham, 4 KiB na cabeça e na cauda. Um `len` que não é
/// múltiplo de 4 KiB leva a última página inteira.
pub fn runs(virt: u64, phys: u64, len: u64) -> impl Iterator<Item = Run> {
    let mut done = 0;
    core::iter::from_fn(move || {
        if done >= len {
            return None;
        }
        let (virt, phys) = (virt + done, phys + done);
        let large = len - done >= LARGE_PAGE_SIZE
            && virt.is_multiple_of(LARGE_PAGE_SIZE)
            && phys.is_multiple_of(LARGE_PAGE_SIZE);
        let run = Run { virt, phys, large };
        done += run.size();
        Some(run)
    })
}

/// Duas faixas `[início, fim)` têm algum byte em comum; faixa vazia não toca nada.
pub const fn overlaps(a: (u64, u64), b: (u64, u64)) -> bool {
    a.0 < a.1 && b.0 < b.1 && a.0 < b.1 && b.0 < a.1
}

/// Junta faixas `[início, fim)` em ordem crescente de início que se tocam ou se
/// sobrepõem: o mapa de memória do bootloader dá regiões vizinhas de tipos que o
/// kernel trata igual. Faixa vazia some.
pub fn coalesce(ranges: impl IntoIterator<Item = (u64, u64)>) -> impl Iterator<Item = (u64, u64)> {
    let mut ranges = ranges.into_iter().filter(|(start, end)| start < end);
    let mut pending = ranges.next();
    core::iter::from_fn(move || {
        let (mut first, mut last) = pending.take()?;
        for (start, end) in ranges.by_ref() {
            if start <= last {
                first = first.min(start);
                last = last.max(end);
            } else {
                pending = Some((start, end));
                break;
            }
        }
        Some((first, last))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1 << 20;
    const VIRT: u64 = 0xffff_8000_0000_0000;

    fn count(virt: u64, phys: u64, len: u64) -> (u64, u64) {
        runs(virt, phys, len).fold((0, 0), |(large, small), run| {
            (large + u64::from(run.large), small + u64::from(!run.large))
        })
    }

    #[test]
    fn a_module_over_the_kernel_image_overlaps_it() {
        let image = (0x20_0000, 0x30_0000);
        assert!(overlaps((0x2f_f000, 0x30_1000), image));
        assert!(overlaps((0x1f_f000, 0x20_1000), image));
        assert!(overlaps((0x10_0000, 0x40_0000), image));
        assert!(overlaps(image, image));
    }

    #[test]
    fn touching_or_empty_ranges_do_not_overlap() {
        assert!(!overlaps((0x10_0000, 0x20_0000), (0x20_0000, 0x30_0000)));
        assert!(!overlaps((0x30_0000, 0x40_0000), (0x20_0000, 0x30_0000)));
        assert!(!overlaps((0x25_0000, 0x25_0000), (0x20_0000, 0x30_0000)));
    }

    #[test]
    fn an_aligned_region_is_all_2_mib_pages() {
        assert_eq!(count(VIRT, 0x4000_0000, 8 * MIB), (4, 0));
        assert_eq!(count(VIRT, 0, 2 * MIB), (1, 0));
    }

    #[test]
    fn a_region_smaller_than_2_mib_is_all_4_kib_pages() {
        assert_eq!(count(VIRT, 0x4000_0000, 2 * MIB - PAGE_SIZE), (0, 511));
        assert_eq!(count(VIRT, 0x4000_0000, PAGE_SIZE), (0, 1));
        assert_eq!(count(VIRT, 0x4000_0000, 0), (0, 0));
    }

    #[test]
    fn an_unaligned_head_and_tail_take_4_kib_pages_around_the_large_ones() {
        let (start, len) = (0x4000_0000 + 3 * PAGE_SIZE, 6 * MIB);
        let (virt, phys) = (VIRT + start, start);
        let all: Vec<_> = runs(virt, phys, len).collect();
        let head = (2 * MIB - 3 * PAGE_SIZE) / PAGE_SIZE;
        let tail = 3;
        let large = all.iter().filter(|run| run.large).count() as u64;
        assert_eq!((large, all.len() as u64 - large), (2, head + tail));
        let first_large = all.iter().position(|run| run.large).unwrap();
        assert_eq!(first_large as u64, head);
        assert_eq!(all[first_large].phys, 0x4000_0000 + 2 * MIB);
    }

    #[test]
    fn runs_tile_the_region_without_gaps_or_overlap() {
        for (virt, phys, len) in [
            (VIRT + 5 * PAGE_SIZE, 5 * PAGE_SIZE, 9 * MIB + 7 * PAGE_SIZE),
            (VIRT, 0x20_0000, 4 * MIB),
            (VIRT + 0x1000, 0x20_1000, 5 * MIB),
        ] {
            let mut next = (virt, phys);
            for run in runs(virt, phys, len) {
                assert_eq!((run.virt, run.phys), next);
                if run.large {
                    assert!(run.virt % LARGE_PAGE_SIZE == 0 && run.phys % LARGE_PAGE_SIZE == 0);
                }
                next = (next.0 + run.size(), next.1 + run.size());
            }
            assert_eq!(next.0 - virt, len);
        }
    }

    #[test]
    fn virtual_and_physical_that_disagree_on_alignment_never_use_large_pages() {
        assert_eq!(count(VIRT, 0x4000_0000 + PAGE_SIZE, 8 * MIB), (0, 2048));
        assert_eq!(count(VIRT + PAGE_SIZE, 0x4000_0000, 8 * MIB), (0, 2048));
    }

    #[test]
    fn a_length_that_is_not_a_page_multiple_takes_the_whole_last_page() {
        let all: Vec<_> = runs(VIRT, 0x1000, PAGE_SIZE + 1).collect();
        assert_eq!(all.len(), 2);
        assert!(!all[1].large);
    }

    fn merged(ranges: &[(u64, u64)]) -> Vec<(u64, u64)> {
        coalesce(ranges.iter().copied()).collect()
    }

    #[test]
    fn adjacent_ranges_become_one() {
        assert_eq!(merged(&[(0, 10), (10, 20), (20, 25)]), [(0, 25)]);
    }

    #[test]
    fn overlapping_ranges_become_one() {
        assert_eq!(merged(&[(0, 10), (5, 20)]), [(0, 20)]);
        assert_eq!(merged(&[(0, 30), (5, 20)]), [(0, 30)]);
    }

    #[test]
    fn ranges_with_a_gap_stay_apart_and_empty_ones_vanish() {
        assert_eq!(merged(&[(0, 10), (11, 20)]), [(0, 10), (11, 20)]);
        assert_eq!(merged(&[(5, 5), (0, 10), (10, 10), (12, 12)]), [(0, 10)]);
        assert_eq!(merged(&[]), []);
        assert_eq!(merged(&[(3, 3)]), []);
    }

    #[test]
    fn a_range_after_a_gap_starts_a_new_run_that_can_still_grow() {
        assert_eq!(
            merged(&[(0, 4), (8, 12), (12, 16), (20, 30)]),
            [(0, 4), (8, 16), (20, 30)]
        );
    }
}
