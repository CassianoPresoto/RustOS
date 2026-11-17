//! Janela de endereços virtuais distribuída em ordem, sem devolução.

use rustos_hal::{VirtAddr, addr::align_up};

use crate::FRAME_SIZE;

/// Tamanho de uma entrada do nível mais alto da tabela (512 GiB), nas duas ISAs.
pub const TOP_LEVEL_SLOT: u64 = 1 << 39;

/// Primeira entrada livre do nível mais alto, de cima para baixo, abaixo da 511
/// (onde fica a imagem do kernel). O mapa direto e a imagem, que o bootloader
/// sorteia, entram em `occupied` como `[início, fim)`.
pub fn free_top_level_slot(occupied: &[(u64, u64)]) -> Option<VirtAddr> {
    (256..511_u64).rev().find_map(|index| {
        let start = 0xffff_0000_0000_0000 + (index << 39);
        let end = start + TOP_LEVEL_SLOT;
        let taken = occupied.iter().any(|&(from, to)| from < end && start < to);
        (!taken).then_some(VirtAddr::new(start))
    })
}

#[derive(Debug)]
pub struct AddressWindow {
    next: u64,
    end: u64,
}

impl AddressWindow {
    pub const fn new(start: VirtAddr, len: u64) -> Self {
        Self {
            next: start.as_u64(),
            end: start.as_u64().saturating_add(len),
        }
    }

    /// Reserva `len` bytes arredondados para páginas inteiras.
    pub fn take(&mut self, len: u64) -> Option<VirtAddr> {
        let len = align_up(len.max(1), FRAME_SIZE)?;
        let start = self.next;
        let end = start.checked_add(len)?;
        if end > self.end {
            return None;
        }
        self.next = end;
        Some(VirtAddr::new(start))
    }

    pub const fn remaining(&self) -> u64 {
        self.end - self.next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hands_out_consecutive_page_aligned_ranges() {
        let mut window = AddressWindow::new(VirtAddr::new(0x10000), 0x4000);
        assert_eq!(window.take(1), Some(VirtAddr::new(0x10000)));
        assert_eq!(window.take(0x1001), Some(VirtAddr::new(0x11000)));
        assert_eq!(window.remaining(), 0x1000);
    }

    #[test]
    fn free_slot_is_the_highest_one_below_the_image() {
        assert_eq!(
            free_top_level_slot(&[]),
            Some(VirtAddr::new(0xffff_ff00_0000_0000))
        );
    }

    #[test]
    fn free_slot_skips_any_slot_the_direct_map_touches() {
        let tail_of_509 = (0xffff_ff00_0000_0000 - 0x1000, 0xffff_ff00_0000_0000);
        let head_of_510 = (0xffff_ff00_0000_0000, 0xffff_ff00_0000_1000);
        let slot_509 = Some(VirtAddr::new(0xffff_fe80_0000_0000));
        assert_eq!(free_top_level_slot(&[head_of_510]), slot_509);
        let slot_508 = Some(VirtAddr::new(0xffff_fe00_0000_0000));
        assert_eq!(free_top_level_slot(&[tail_of_509, head_of_510]), slot_508);
    }

    #[test]
    fn free_slot_gives_up_when_everything_is_taken() {
        assert_eq!(
            free_top_level_slot(&[(0xffff_8000_0000_0000, u64::MAX)]),
            None
        );
    }

    #[test]
    fn refuses_when_exhausted() {
        let mut window = AddressWindow::new(VirtAddr::new(0x10000), 0x1000);
        assert!(window.take(0x1000).is_some());
        assert_eq!(window.take(1), None);
    }
}
