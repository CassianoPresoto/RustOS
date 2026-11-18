//! Liga `core/mm` à memória de verdade: o único lugar que transforma endereço
//! físico em ponteiro, pelo mapa direto que o bootloader montou.

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU64, Ordering};
use core::{ptr, slice};

use rustos_hal::{
    BootInfo, FrameSource, MapError, MemoryRegion, PAGE_SIZE, PageFlags, PageMapper, PhysAddr,
    RegionKind, VirtAddr,
};
use rustos_mm::{
    AddressWindow, BuddyAllocator, FrameAllocator, FrameError, MemorySummary, PhysFrame,
    find_free_block, free_top_level_slot, summarize,
};

use crate::boot::BootRecord;
use crate::frames::{self, Frames};
use crate::heap;
use crate::platform;
use crate::stack::Stack;
use crate::sync::IrqLock;

/// Janelas de dispositivos, de pilhas e de módulos: 1 GiB cada, vazias até `place_windows`.
const WINDOW_LEN: u64 = 1 << 30;

/// O deslocamento do mapa direto, guardado por `Memory::init` para quem converte frame em ponteiro.
static DIRECT_MAP_OFFSET: AtomicU64 = AtomicU64::new(0);

static DEVICES: IrqLock<AddressWindow> = IrqLock::new(AddressWindow::new(VirtAddr::new(0), 0));
static STACKS: IrqLock<AddressWindow> = IrqLock::new(AddressWindow::new(VirtAddr::new(0), 0));
static MODULES: IrqLock<AddressWindow> = IrqLock::new(AddressWindow::new(VirtAddr::new(0), 0));

#[derive(Debug)]
pub enum MemoryError {
    NoUsableMemory,
    NoRoomForMetadata,
    /// Nenhuma entrada do nível mais alto livre para as janelas.
    NoAddressSpace,
    Frames(#[allow(dead_code, reason = "lido pelo Debug da mensagem de pânico")] FrameError),
}

#[derive(Debug)]
pub struct Memory {
    pub frames: Frames,
    pub summary: MemorySummary,
    pub metadata_bytes: u64,
    direct_map_offset: u64,
}

impl Memory {
    /// Instala os globais de frames e de heap.
    /// # Safety
    /// Chamar uma vez. As regiões utilizáveis do mapa estão livres e acessíveis
    /// em `físico + direct_map_offset`.
    pub unsafe fn init(boot: &impl BootInfo) -> Result<Self, MemoryError> {
        let summary = summarize(boot.memory_regions());
        let (base, _) = summary.managed_span.ok_or(MemoryError::NoUsableMemory)?;
        let frames =
            usize::try_from(summary.span_frames()).map_err(|_| MemoryError::NoUsableMemory)?;
        let words = BuddyAllocator::words_needed(frames);
        let metadata_bytes = words as u64 * 8;
        let block = find_free_block(boot.memory_regions(), metadata_bytes)
            .ok_or(MemoryError::NoRoomForMetadata)?;

        // SAFETY: o bloco é utilizável e ninguém o usa ainda, pelo contrato da função.
        let storage = unsafe { direct_map_slice(block, words, boot.direct_map_offset()) };
        let mut allocator =
            BuddyAllocator::new(storage, base, frames).map_err(MemoryError::Frames)?;
        for region in boot.memory_regions().filter(MemoryRegion::is_usable) {
            allocator.release_range(region.start, region.len);
        }
        allocator.reserve_range(block, metadata_bytes);
        let frames = frames::install(allocator);
        DIRECT_MAP_OFFSET.store(boot.direct_map_offset(), Ordering::Relaxed);
        // SAFETY: o contrato desta função põe todo frame utilizável no mapa direto.
        unsafe { heap::install(frames, boot.direct_map_offset()) };
        Ok(Self {
            frames,
            summary,
            metadata_bytes,
            direct_map_offset: boot.direct_map_offset(),
        })
    }

    /// Põe as janelas de dispositivos, de pilhas e de módulos, uma depois da outra, na entrada do
    /// nível mais alto que `occupied` (mapa direto e imagem, sorteados) não toca.
    pub fn place_windows(occupied: &[(u64, u64)]) -> Result<(), MemoryError> {
        let base = free_top_level_slot(occupied).ok_or(MemoryError::NoAddressSpace)?;
        let stacks = VirtAddr::new(base.as_u64() + WINDOW_LEN);
        crate::addr::set_windows(base.as_u64(), WINDOW_LEN);
        *DEVICES.lock() = AddressWindow::new(base, WINDOW_LEN);
        *STACKS.lock() = AddressWindow::new(stacks, WINDOW_LEN);
        *MODULES.lock() =
            AddressWindow::new(VirtAddr::new(stacks.as_u64() + WINDOW_LEN), WINDOW_LEN);
        Ok(())
    }

    /// Devolve ao alocador as regiões `BootloaderReclaimable` e atualiza `record`.
    /// Só depois de tabelas, pilha e GDT próprias: nada pode apontar para elas.
    /// Devolve os bytes recuperados.
    pub fn reclaim_bootloader(&mut self, record: &mut BootRecord) -> u64 {
        for region in record
            .memory_regions()
            .filter(|region| region.kind == RegionKind::BootloaderReclaimable)
        {
            self.frames.release_range(region.start, region.len);
        }
        let bytes = record.mark_reclaimed();
        self.summary = summarize(record.memory_regions());
        bytes
    }

    /// Pilha de `pages` páginas com uma página sem mapeamento logo abaixo: estouro
    /// vira page fault em vez de corromper memória. Devolve o topo (fim exclusivo).
    pub fn map_stack(
        &mut self,
        mapper: &mut impl PageMapper,
        pages: u64,
    ) -> Result<VirtAddr, MapError> {
        map_stack(&mut self.frames, mapper, pages)
    }

    /// `map_stack` com a pilha pintada para medir o pico ([`Stack`]).
    pub fn map_stack_painted(
        &mut self,
        mapper: &mut impl PageMapper,
        pages: u64,
    ) -> Result<Stack, MapError> {
        map_stack_painted(&mut self.frames, mapper, pages)
    }

    /// Fatia de `len` valores `fill` em frames contíguos, para sempre do chamador.
    pub fn alloc_slice<T: Copy>(&mut self, len: usize, fill: T) -> Option<&'static mut [T]> {
        const { assert!(align_of::<T>() as u64 <= PAGE_SIZE) };
        let bytes = len.checked_mul(size_of::<T>())?;
        let frames = bytes.div_ceil(usize::try_from(PAGE_SIZE).ok()?).max(1);
        let first = self.frames.allocate_contiguous(frames)?;
        let virt = usize::try_from(first.start().as_u64() + self.direct_map_offset).ok()?;
        let start = ptr::with_exposed_provenance_mut::<MaybeUninit<T>>(virt);
        // SAFETY: frames recém-saídos do alocador, no mapa direto (contrato do `init`).
        let slots = unsafe { slice::from_raw_parts_mut(start, len) };
        for slot in slots.iter_mut() {
            slot.write(fill);
        }
        // SAFETY: todo elemento acabou de ser inicializado com `fill`.
        Some(unsafe { slice::from_raw_parts_mut(start.cast::<T>(), len) })
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

/// Pilha de `pages` páginas com guarda abaixo, na janela de pilhas. Quem chama
/// serializa o `mapper`: as tabelas não têm lock próprio.
pub fn map_stack(
    frames: &mut Frames,
    mapper: &mut impl PageMapper,
    pages: u64,
) -> Result<VirtAddr, MapError> {
    let span = (pages + 1) * PAGE_SIZE;
    let base = STACKS
        .lock()
        .take(span)
        .ok_or(MapError::OutOfAddressSpace)?;
    for page in 1..=pages {
        let frame = frames.allocate().ok_or(MapError::OutOfFrames)?;
        let virt = VirtAddr::new(base.as_u64() + page * PAGE_SIZE);
        mapper.map_page(virt, frame.start(), PageFlags::DATA, frames)?;
    }
    Ok(VirtAddr::new(base.as_u64() + span))
}

/// `map_stack` com a pilha pintada para medir o pico ([`Stack`]).
pub fn map_stack_painted(
    frames: &mut Frames,
    mapper: &mut impl PageMapper,
    pages: u64,
) -> Result<Stack, MapError> {
    let top = map_stack(frames, mapper, pages)?;
    // SAFETY: `map_stack` mapeou as `pages` páginas abaixo de `top`, alinhadas, para sempre.
    let stack = unsafe { Stack::new(top, pages) };
    // SAFETY: a pilha só vai a quem a recebe daqui.
    unsafe { stack.paint() };
    Ok(stack)
}

/// Mapeia registradores de um dispositivo na janela de dispositivos.
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

/// Mapeia um módulo do bootloader na janela de módulos, só leitura e sem execução
/// (`rustos_mm::map_module`). `phys` é alinhado a uma página.
pub fn map_module(
    mapper: &mut impl PageMapper,
    frames: &mut impl FrameSource,
    phys: PhysAddr,
    len: u64,
) -> Result<VirtAddr, MapError> {
    rustos_mm::map_module(
        |bytes| MODULES.lock().take(bytes),
        mapper,
        frames,
        phys,
        len,
    )
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

#[allow(
    clippy::cast_possible_truncation,
    reason = "o kernel só existe em 64 bits"
)]
const FRAME_BYTES: usize = PAGE_SIZE as usize;

/// O ponteiro de um frame pelo mapa direto.
fn frame_ptr(phys: PhysAddr) -> *mut u8 {
    let offset = DIRECT_MAP_OFFSET.load(Ordering::Relaxed);
    ptr::with_exposed_provenance_mut(
        usize::try_from(phys.as_u64() + offset).expect("endereço cabe em usize"),
    )
}

/// Zera o frame inteiro, pelo mapa direto.
pub fn zero_frame(phys: PhysAddr) {
    // SAFETY: todo frame do alocador é acessível no mapa direto (contrato de `Memory::init`) e o
    // chamador o tem só para si.
    unsafe { frame_ptr(phys).write_bytes(0, FRAME_BYTES) };
}

/// O frame tem código que o usuário vai executar: torna a escrita visível ao cache de instrução.
pub fn sync_code_frame(phys: PhysAddr) {
    platform::sync_icache(frame_ptr(phys).addr(), FRAME_BYTES);
}

/// Copia `bytes` para o início do frame (que o chamador zerou), pelo mapa direto.
///
/// # Safety
/// `bytes.len() <= PAGE_SIZE` e o frame é só do chamador.
pub unsafe fn fill_frame(phys: PhysAddr, bytes: &[u8]) {
    // SAFETY: o contrato desta função; os dois não se sobrepõem.
    unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), frame_ptr(phys), bytes.len()) };
}
