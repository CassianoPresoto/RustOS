//! Liga `core/mm` à memória de verdade: o único lugar que transforma endereço
//! físico em ponteiro, pelo mapa direto que o bootloader montou.

use core::{ptr, slice};

use rustos_hal::{
    BootInfo, FrameSource, MapError, MemoryRegion, PAGE_SIZE, PageFlags, PageMapper, PhysAddr,
    VirtAddr,
};
use rustos_mm::{
    AddressWindow, BitmapFrameAllocator, FrameError, MemorySummary, PhysFrame, find_free_block,
    summarize,
};
use rustos_sync::SpinLock;

/// Janela de dispositivos: entrada 510 do nível mais alto, livre no layout do Limine.
#[cfg_attr(
    target_arch = "x86_64",
    allow(dead_code, reason = "a COM1 é porta de E/S (DEBT-0004)")
)]
const DEVICE_WINDOW: VirtAddr = VirtAddr::new(0xffff_ff00_0000_0000);
#[cfg_attr(
    target_arch = "x86_64",
    allow(dead_code, reason = "a COM1 é porta de E/S (DEBT-0004)")
)]
const DEVICE_WINDOW_LEN: u64 = 1 << 30;

#[cfg_attr(
    target_arch = "x86_64",
    allow(dead_code, reason = "a COM1 é porta de E/S (DEBT-0004)")
)]
static DEVICES: SpinLock<AddressWindow> =
    SpinLock::new(AddressWindow::new(DEVICE_WINDOW, DEVICE_WINDOW_LEN));

#[derive(Debug)]
pub enum MemoryError {
    NoUsableMemory,
    NoRoomForBitmap,
    Frames(#[allow(dead_code, reason = "lido pelo Debug da mensagem de pânico")] FrameError),
}

#[derive(Debug)]
pub struct Memory {
    pub frames: BitmapFrameAllocator<'static>,
    pub summary: MemorySummary,
    pub bitmap_bytes: u64,
}

impl Memory {
    /// # Safety
    /// Chamar uma vez. As regiões utilizáveis do mapa estão livres e acessíveis
    /// em `físico + direct_map_offset`.
    pub unsafe fn init(boot: &impl BootInfo) -> Result<Self, MemoryError> {
        let summary = summarize(boot.memory_regions());
        let (base, _) = summary.usable_span.ok_or(MemoryError::NoUsableMemory)?;
        let frames =
            usize::try_from(summary.span_frames()).map_err(|_| MemoryError::NoUsableMemory)?;
        let words = BitmapFrameAllocator::words_needed(frames);
        let bitmap_bytes = words as u64 * 8;
        let block = find_free_block(boot.memory_regions(), bitmap_bytes)
            .ok_or(MemoryError::NoRoomForBitmap)?;

        // SAFETY: o bloco é utilizável e ninguém o usa ainda, pelo contrato da função.
        let storage = unsafe { direct_map_slice(block, words, boot.direct_map_offset()) };
        let mut allocator =
            BitmapFrameAllocator::new(storage, base, frames).map_err(MemoryError::Frames)?;
        for region in boot.memory_regions().filter(MemoryRegion::is_usable) {
            allocator.release_range(region.start, region.len);
        }
        allocator.reserve_range(block, bitmap_bytes);
        Ok(Self {
            frames: allocator,
            summary,
            bitmap_bytes,
        })
    }

    /// Pega um frame, escreve e relê pelo mapa direto, e o devolve.
    ///
    /// # Safety
    /// Todo frame do alocador é acessível em `físico + direct_map_offset`.
    pub unsafe fn self_test(&mut self, direct_map_offset: u64) -> bool {
        const PATTERN: u64 = 0x5275_7374_4f53_2121;
        let Some(frame) = self.frames.allocate() else {
            return false;
        };
        let words = usize::try_from(PAGE_SIZE / 8).unwrap_or(0);
        // SAFETY: o frame acabou de sair do alocador; é nosso até ser devolvido.
        let page = unsafe { direct_map_slice(frame.start(), words, direct_map_offset) };
        page.fill(PATTERN);
        let intact = page.iter().all(|&word| word == PATTERN);
        intact
            && self
                .frames
                .deallocate(PhysFrame::containing(frame.start()))
                .is_ok()
    }
}

/// Mapeia registradores de um dispositivo na janela de dispositivos.
#[cfg_attr(
    target_arch = "x86_64",
    allow(dead_code, reason = "a COM1 é porta de E/S (DEBT-0004)")
)]
pub fn map_device(
    mapper: &mut impl PageMapper,
    frames: &mut impl FrameSource,
    phys: PhysAddr,
    len: u64,
) -> Result<VirtAddr, MapError> {
    let first = phys.align_down(PAGE_SIZE);
    let offset = phys.as_u64() - first.as_u64();
    let span = len + offset;
    let virt = DEVICES
        .lock()
        .take(span)
        .ok_or(MapError::OutOfAddressSpace)?;
    for page in 0..span.div_ceil(PAGE_SIZE) {
        let delta = page * PAGE_SIZE;
        let page_virt = VirtAddr::new(virt.as_u64() + delta);
        let page_phys = PhysAddr::new(first.as_u64() + delta);
        mapper.map_page(page_virt, page_phys, PageFlags::DEVICE, frames)?;
    }
    Ok(VirtAddr::new(virt.as_u64() + offset))
}

/// # Safety
/// `[phys, phys + words * 8)` é memória livre, exclusiva de quem chama por `'static`,
/// e acessível em `físico + direct_map_offset`.
unsafe fn direct_map_slice(
    phys: PhysAddr,
    words: usize,
    direct_map_offset: u64,
) -> &'static mut [u64] {
    let virt = usize::try_from(phys.as_u64() + direct_map_offset).expect("endereço cabe em usize");
    let start = ptr::with_exposed_provenance_mut::<u64>(virt);
    // SAFETY: região exclusiva e mapeada, pelo contrato da função.
    unsafe { slice::from_raw_parts_mut(start, words) }
}
