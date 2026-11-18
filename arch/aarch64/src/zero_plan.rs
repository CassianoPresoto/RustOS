//! Como dividir uma faixa para zerar com `DC ZVA`. Puro: a instrução fica em `zero.rs`.

/// Unidade do recuo e do que sobra fora de bloco: o laço de `STP` grava 64 bytes por volta.
pub const LINE: usize = 64;

/// `DCZID_EL0.DZP` (bit 4): ligado, `DC ZVA` é proibido.
const DCZID_DZP: u64 = 1 << 4;
/// `DCZID_EL0.BS` (bits 3:0): log2 do bloco em palavras de 4 bytes.
const DCZID_BS_MASK: u64 = 0xf;

/// Bytes que um `DC ZVA` zera, ou `None` se o `DZP` o proíbe (`clear_page.S` do Linux:
/// `tbnz x1, #4` e `4 << (x1 & 0xf)`). Um bloco menor que `LINE` também recua.
pub const fn zva_block_bytes(dczid: u64) -> Option<usize> {
    if dczid & DCZID_DZP != 0 {
        return None;
    }
    let block = 4_usize << (dczid & DCZID_BS_MASK);
    if block < LINE { None } else { Some(block) }
}

/// A faixa `[addr, addr + len)` em três pedaços contíguos, em bytes: `head` até o
/// primeiro bloco alinhado, `body` em blocos inteiros e `tail` depois do último.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Split {
    pub head: usize,
    pub body: usize,
    pub tail: usize,
}

/// Divide a faixa para `block` bytes (potência de 2, múltiplo de `LINE`). `addr` e `len`
/// são múltiplos de `LINE`. Sem bloco inteiro alinhado, tudo vira `head`.
pub const fn split(addr: usize, len: usize, block: usize) -> Split {
    let head = addr.wrapping_neg() & (block - 1);
    if head >= len || len - head < block {
        return Split {
            head: len,
            body: 0,
            tail: 0,
        };
    }
    let body = (len - head) & !(block - 1);
    Split {
        head,
        body,
        tail: len - head - body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dzp_forbids_the_instruction() {
        assert_eq!(zva_block_bytes(1 << 4), None);
        assert_eq!(zva_block_bytes((1 << 4) | 4), None);
    }

    #[test]
    fn block_size_is_four_bytes_shifted_by_bs() {
        assert_eq!(zva_block_bytes(4), Some(64));
        assert_eq!(zva_block_bytes(5), Some(128));
        assert_eq!(zva_block_bytes(9), Some(2048));
        assert_eq!(zva_block_bytes(0xffff_ffff_ffff_ffe4), Some(64));
    }

    #[test]
    fn a_block_smaller_than_a_line_falls_back() {
        assert_eq!(zva_block_bytes(0), None);
        assert_eq!(zva_block_bytes(3), None);
    }

    #[test]
    fn an_aligned_page_is_all_body() {
        for block in [64, 128, 256, 1024, 2048, 4096] {
            let plan = split(0x4000, 4096, block);
            assert_eq!(
                plan,
                Split {
                    head: 0,
                    body: 4096,
                    tail: 0
                },
                "block {block}"
            );
        }
    }

    #[test]
    fn head_and_tail_keep_the_body_on_block_boundaries() {
        let plan = split(0x1040, 4096, 128);
        assert_eq!(
            plan,
            Split {
                head: 64,
                body: 3968,
                tail: 64
            }
        );
        assert!((0x1040 + plan.head).is_multiple_of(128));
        assert_eq!(plan.head + plan.body + plan.tail, 4096);
    }

    #[test]
    fn a_range_shorter_than_a_block_is_all_head() {
        assert_eq!(
            split(0x40, 64, 128),
            Split {
                head: 64,
                body: 0,
                tail: 0
            }
        );
        assert_eq!(
            split(0x80, 64, 128),
            Split {
                head: 64,
                body: 0,
                tail: 0
            }
        );
        assert_eq!(
            split(0x40, 128, 128),
            Split {
                head: 128,
                body: 0,
                tail: 0
            }
        );
    }

    #[test]
    fn every_split_covers_the_range_without_touching_outside_it() {
        for block in [64_usize, 128, 512, 2048] {
            for addr in (0..64).map(|i| i * LINE) {
                for len in (1..80).map(|i| i * LINE) {
                    let plan = split(addr, len, block);
                    assert_eq!(plan.head + plan.body + plan.tail, len);
                    assert_eq!(plan.body % block, 0);
                    assert!((addr + plan.head).is_multiple_of(block) || plan.body == 0);
                    assert!(plan.tail < block);
                }
            }
        }
    }
}
