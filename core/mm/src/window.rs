//! Janela de endereços virtuais distribuída em ordem, sem devolução.

use rustos_hal::{VirtAddr, addr::align_up};

use crate::FRAME_SIZE;

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
    fn refuses_when_exhausted() {
        let mut window = AddressWindow::new(VirtAddr::new(0x10000), 0x1000);
        assert!(window.take(0x1000).is_some());
        assert_eq!(window.take(1), None);
    }
}
