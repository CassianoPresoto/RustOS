//! Clamp de índice contra o Spectre v1 (`array_index_nospec` do Linux), sem desvio. Puro.

/// A máscara do clamp: todo bit ligado se `index < size`, zero caso contrário. `size` até
/// `isize::MAX`. O `black_box` impede o compilador de trocá-la por desvio; o assembly é conferido
/// à mão (DEBT-0105).
#[inline]
pub fn array_index_mask_nospec(index: usize, size: usize) -> usize {
    let index = core::hint::black_box(index);
    let sign =
        (index | size.wrapping_sub(1).wrapping_sub(index)).cast_signed() >> (usize::BITS - 1);
    (!sign).cast_unsigned()
}

/// `index` se `index < size`, 0 caso contrário: a CPU que especula além da conferência de
/// limite lê no máximo o elemento 0. Chame **depois** da conferência, nunca no lugar dela.
#[inline]
pub fn array_index_nospec(index: usize, size: usize) -> usize {
    index & array_index_mask_nospec(index, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_inside_the_table_is_untouched() {
        for size in [1_usize, 2, 7, 64, 0x43] {
            for index in 0..size {
                assert_eq!(array_index_nospec(index, size), index, "{index} of {size}");
            }
        }
    }

    #[test]
    fn an_index_outside_the_table_becomes_zero() {
        for size in [1_usize, 2, 7, 64, 0x43] {
            for index in size..size + 70 {
                assert_eq!(array_index_nospec(index, size), 0, "{index} of {size}");
            }
        }
    }

    #[test]
    fn the_edges_of_the_word_are_outside_too() {
        let size = 0x43;
        for index in [
            usize::MAX,
            usize::MAX - 1,
            isize::MAX as usize,
            isize::MAX as usize + 1,
            1 << 63 | 5,
        ] {
            assert_eq!(array_index_nospec(index, size), 0, "{index:#x}");
        }
    }

    #[test]
    fn the_largest_size_the_contract_allows_still_clamps() {
        let size = isize::MAX as usize;
        assert_eq!(array_index_nospec(size - 1, size), size - 1);
        assert_eq!(array_index_nospec(size, size), 0);
        assert_eq!(array_index_nospec(0, size), 0);
    }

    #[test]
    fn the_mask_is_all_ones_inside_and_zero_outside() {
        assert_eq!(array_index_mask_nospec(0, 1), usize::MAX);
        assert_eq!(array_index_mask_nospec(66, 67), usize::MAX);
        assert_eq!(array_index_mask_nospec(67, 67), 0);
        assert_eq!(array_index_mask_nospec(usize::MAX, 67), 0);
    }

    #[test]
    fn an_empty_table_clamps_everything_to_zero() {
        for index in [0, 1, 5, usize::MAX] {
            assert_eq!(array_index_nospec(index, 0), 0);
        }
    }
}
