//! Janela de endereços virtuais distribuída em ordem, sem devolução.

use rustos_hal::{
    FrameSource, MapError, PAGE_SIZE, PageFlags, PageMapper, PhysAddr, VirtAddr, addr::align_up,
};

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

/// Mapeia `[phys, phys + len)`, de um módulo do bootloader, só leitura e sem execução, numa faixa
/// que `take` reserva (em bytes, de páginas inteiras). Devolve o endereço virtual do primeiro byte.
/// `len` leva a última página inteira; sem espaço em `take`, `OutOfAddressSpace`.
pub fn map_module<M: PageMapper, F: FrameSource>(
    take: impl FnOnce(u64) -> Option<VirtAddr>,
    mapper: &mut M,
    frames: &mut F,
    phys: PhysAddr,
    len: u64,
) -> Result<VirtAddr, MapError> {
    let pages = len.div_ceil(PAGE_SIZE);
    let bytes = pages
        .checked_mul(PAGE_SIZE)
        .ok_or(MapError::OutOfAddressSpace)?;
    let virt = take(bytes).ok_or(MapError::OutOfAddressSpace)?;
    for page in 0..pages {
        let delta = page * PAGE_SIZE;
        let page_virt = VirtAddr::new(virt.as_u64() + delta);
        let page_phys = phys.checked_add(delta).ok_or(MapError::OutOfAddressSpace)?;
        mapper.map_page(page_virt, page_phys, PageFlags::RODATA, frames)?;
    }
    Ok(virt)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Grava cada página mapeada; recusa a de número `fail_at`, se houver.
    #[derive(Default)]
    struct Recorder {
        pages: Vec<(u64, u64, PageFlags)>,
        fail_at: Option<usize>,
    }

    impl PageMapper for Recorder {
        fn map_page<F: FrameSource>(
            &mut self,
            virt: VirtAddr,
            phys: PhysAddr,
            flags: PageFlags,
            _: &mut F,
        ) -> Result<(), MapError> {
            if self.fail_at == Some(self.pages.len()) {
                return Err(MapError::OutOfFrames);
            }
            self.pages.push((virt.as_u64(), phys.as_u64(), flags));
            Ok(())
        }

        fn map_large_page<F: FrameSource>(
            &mut self,
            _: VirtAddr,
            _: PhysAddr,
            _: PageFlags,
            _: &mut F,
        ) -> Result<(), MapError> {
            unreachable!("o módulo usa páginas de 4 KiB")
        }
    }

    struct NoFrames;

    impl FrameSource for NoFrames {
        fn allocate_frame(&mut self) -> Option<PhysAddr> {
            None
        }
    }

    fn map(
        window: &mut AddressWindow,
        mapper: &mut Recorder,
        len: u64,
    ) -> Result<VirtAddr, MapError> {
        map_module(
            |bytes| window.take(bytes),
            mapper,
            &mut NoFrames,
            PhysAddr::new(0x40_0000),
            len,
        )
    }

    #[test]
    fn a_module_maps_whole_pages_read_only_and_never_executable() {
        let mut window = AddressWindow::new(VirtAddr::new(0x10_0000), 0x10_0000);
        let mut mapper = Recorder::default();
        assert_eq!(
            map(&mut window, &mut mapper, 0x1001),
            Ok(VirtAddr::new(0x10_0000))
        );
        assert_eq!(
            mapper.pages,
            [
                (0x10_0000, 0x40_0000, PageFlags::RODATA),
                (0x10_1000, 0x40_1000, PageFlags::RODATA),
            ]
        );
        assert!(
            mapper
                .pages
                .iter()
                .all(|&(.., flags)| !flags.writable && !flags.executable)
        );
        assert_eq!(window.remaining(), 0x10_0000 - 0x2000);
    }

    #[test]
    fn a_module_shorter_than_a_page_and_one_of_exactly_a_page_take_one_page() {
        for len in [1, 272, 0x1000] {
            let mut window = AddressWindow::new(VirtAddr::new(0x10_0000), 0x10_0000);
            let mut mapper = Recorder::default();
            assert!(map(&mut window, &mut mapper, len).is_ok());
            assert_eq!(mapper.pages.len(), 1, "{len}");
            assert_eq!(window.remaining(), 0x10_0000 - 0x1000);
        }
    }

    #[test]
    fn a_module_that_does_not_fit_the_window_maps_nothing() {
        let mut window = AddressWindow::new(VirtAddr::new(0x10_0000), 0x2000);
        let mut mapper = Recorder::default();
        assert_eq!(
            map(&mut window, &mut mapper, 0x2001),
            Err(MapError::OutOfAddressSpace)
        );
        assert_eq!(mapper.pages.len(), 0);
        assert_eq!(
            map(&mut window, &mut mapper, 0x2000),
            Ok(VirtAddr::new(0x10_0000))
        );
        assert_eq!(
            map(&mut window, &mut mapper, 1),
            Err(MapError::OutOfAddressSpace)
        );
    }

    #[test]
    fn a_length_that_overflows_the_page_count_is_out_of_address_space() {
        let mut window = AddressWindow::new(VirtAddr::new(0x10_0000), 0x10_0000);
        let mut mapper = Recorder::default();
        assert_eq!(
            map(&mut window, &mut mapper, u64::MAX),
            Err(MapError::OutOfAddressSpace)
        );
        assert_eq!(mapper.pages.len(), 0);
    }

    #[test]
    fn a_physical_range_that_wraps_is_out_of_address_space() {
        let mut window = AddressWindow::new(VirtAddr::new(0x10_0000), 0x10_0000);
        let mut mapper = Recorder::default();
        let result = map_module(
            |bytes| window.take(bytes),
            &mut mapper,
            &mut NoFrames,
            PhysAddr::new(u64::MAX - 0xfff),
            0x2000,
        );
        assert_eq!(result, Err(MapError::OutOfAddressSpace));
    }

    #[test]
    fn a_mapper_error_stops_the_mapping_and_is_reported() {
        let mut window = AddressWindow::new(VirtAddr::new(0x10_0000), 0x10_0000);
        let mut mapper = Recorder {
            fail_at: Some(1),
            ..Recorder::default()
        };
        assert_eq!(
            map(&mut window, &mut mapper, 0x3000),
            Err(MapError::OutOfFrames)
        );
        assert_eq!(mapper.pages.len(), 1);
    }

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
