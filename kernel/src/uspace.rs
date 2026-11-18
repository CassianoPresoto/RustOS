//! O espaço de endereçamento de um processo de usuário: tabelas próprias para a metade baixa
//! (`CR3` ou `TTBR0_EL1`), frames que ele mapeia e a devolução deles, zerados, na morte (SDD 17).

use rustos_hal::abi::{USER_MIN, USER_TOP};
use rustos_hal::{FrameSource, MapError, PAGE_SIZE, PageFlags, PageMapper, PhysAddr, VirtAddr};
use rustos_mm::PhysFrame;

use crate::frames::Frames;
use crate::memory;
use crate::platform::{self, PlatformSpace};

/// Entradas do nível mais alto que a metade de usuário ocupa: `USER_TOP` cabe nas 256 de baixo.
const USER_TOP_ENTRIES: usize = 256;
const _: () = assert!(USER_TOP >> 39 <= USER_TOP_ENTRIES as u64);

#[derive(Debug)]
pub struct UserSpace {
    tables: PlatformSpace,
    frames: Frames,
}

impl UserSpace {
    /// Raiz nova e vazia (no x86_64 com a metade do kernel copiada).
    ///
    /// # Safety
    /// O mapa direto cobre todo frame de `frames`, e esta CPU já usa as tabelas do kernel.
    pub unsafe fn new(frames: Frames, direct_map_offset: u64) -> Result<Self, MapError> {
        let mut source = frames;
        // SAFETY: o contrato desta função.
        let tables = unsafe { platform::new_user_space(&mut source, direct_map_offset) }?;
        Ok(Self { tables, frames })
    }

    /// A raiz que o `CR3` ou o `TTBR0_EL1` recebe.
    pub fn root(&self) -> PhysAddr {
        self.tables.root()
    }

    /// Mapeia `pages` páginas a partir de `virt`, em frames novos e zerados; os primeiros bytes
    /// vêm de `bytes`. A permissão é a do mapeador: W+X é recusado.
    pub fn map(
        &mut self,
        virt: u64,
        bytes: &[u8],
        pages: u64,
        flags: PageFlags,
    ) -> Result<(), MapError> {
        debug_assert!(virt >= USER_MIN && virt.is_multiple_of(PAGE_SIZE));
        let chunk = usize::try_from(PAGE_SIZE).expect("página cabe em usize");
        for page in 0..pages {
            let frame = self.frames.allocate_frame().ok_or(MapError::OutOfFrames)?;
            memory::zero_frame(frame);
            let start = usize::try_from(page).expect("índice cabe em usize") * chunk;
            if start < bytes.len() {
                let part = &bytes[start..bytes.len().min(start + chunk)];
                // SAFETY: `part` cabe numa página e o frame acabou de sair do alocador, só nosso.
                unsafe { memory::fill_frame(frame, part) };
            }
            let at = VirtAddr::new(virt + page * PAGE_SIZE);
            if let Err(error) = self.tables.map_page(at, frame, flags, &mut self.frames) {
                self.release(frame);
                return Err(error);
            }
        }
        Ok(())
    }

    fn release(&self, frame: PhysAddr) {
        memory::zero_frame(frame);
        let _ = self.frames.deallocate(PhysFrame::containing(frame));
    }

    /// A metade do kernel da raiz é a da raiz do kernel.
    pub fn kernel_half_in_sync(&self) -> bool {
        platform::kernel_half_in_sync(&self.tables)
    }

    /// Devolve ao alocador todo frame de dado, toda tabela e a raiz, zerados. Antes disso nenhuma
    /// CPU pode ter a raiz carregada.
    pub fn destroy(mut self) {
        let frames = self.frames;
        self.tables.release_user_half(USER_TOP_ENTRIES, |frame| {
            memory::zero_frame(frame);
            let _ = frames.deallocate(PhysFrame::containing(frame));
        });
    }
}
