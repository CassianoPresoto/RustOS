//! O espaço de endereçamento de um processo de usuário: tabelas próprias para a metade baixa
//! (`CR3` ou `TTBR0_EL1`), frames que ele mapeia e a devolução deles, zerados, na morte (SDD 17).

#[cfg(feature = "selftest")]
use rustos_hal::AddressSpace;
use rustos_hal::abi::{USER_MIN, USER_TOP};
use rustos_hal::{FrameSource, MapError, PAGE_SIZE, PageFlags, PageMapper, PhysAddr, VirtAddr};
use rustos_mm::PhysFrame;

use crate::frames::Frames;
use crate::memory;
use crate::platform::{self, PlatformSpace};

/// Entradas do nível mais alto que a metade de usuário ocupa: `USER_TOP` cabe nas 256 de baixo.
const USER_TOP_ENTRIES: usize = 256;
const _: () = assert!(USER_TOP >> 39 <= USER_TOP_ENTRIES as u64);

/// Quantos frames de dados `map_at` ainda pode alocar antes de falhar com `OutOfFrames`; `u64::MAX`
/// é sem limite. Só o modo de teste o baixa, para provar a devolução numa falha tardia.
#[cfg(feature = "selftest")]
static FRAME_BUDGET: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(u64::MAX);

/// Limita os frames de dados que as próximas chamadas de `map_at` alocam (`None`: sem limite).
#[cfg(feature = "selftest")]
pub fn set_frame_budget(frames: Option<u64>) {
    FRAME_BUDGET.store(
        frames.unwrap_or(u64::MAX),
        core::sync::atomic::Ordering::Relaxed,
    );
}

/// Gasta um frame do orçamento; `false` se acabou.
#[cfg(feature = "selftest")]
fn spend_budget() -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    let left = FRAME_BUDGET.load(Relaxed);
    if left == u64::MAX {
        return true;
    }
    if left == 0 {
        return false;
    }
    FRAME_BUDGET.store(left - 1, Relaxed);
    true
}

/// Frames de um espaço: os que guardam dados do programa e os que guardam tabelas.
#[cfg_attr(
    not(feature = "selftest"),
    allow(dead_code, reason = "lido só pelo modo de teste")
)]
#[derive(Clone, Copy, Debug, Default)]
pub struct SpaceStats {
    pub data: u64,
    pub tables: u64,
}

/// Conta os frames que o mapeador gasta em tabelas.
struct Counted<'a> {
    frames: &'a mut Frames,
    taken: u64,
}

impl FrameSource for Counted<'_> {
    fn allocate_frame(&mut self) -> Option<PhysAddr> {
        let frame = self.frames.allocate_frame();
        self.taken += u64::from(frame.is_some());
        frame
    }
}

#[derive(Debug)]
pub struct UserSpace {
    tables: PlatformSpace,
    frames: Frames,
    data_frames: u64,
    table_frames: u64,
}

impl UserSpace {
    /// Raiz nova e vazia (no x86_64 com a metade do kernel copiada).
    ///
    /// # Safety
    /// O mapa direto cobre todo frame de `frames`, e esta CPU já usa as tabelas do kernel.
    pub unsafe fn new(frames: Frames, direct_map_offset: u64) -> Result<Self, MapError> {
        let mut copy = frames;
        let mut source = Counted {
            frames: &mut copy,
            taken: 0,
        };
        // SAFETY: o contrato desta função.
        let tables = unsafe { platform::new_user_space(&mut source, direct_map_offset) }?;
        let table_frames = source.taken;
        Ok(Self {
            tables,
            frames,
            data_frames: 0,
            table_frames,
        })
    }

    /// A raiz que o `CR3` ou o `TTBR0_EL1` recebe.
    pub fn root(&self) -> PhysAddr {
        self.tables.root()
    }

    /// Mapeia `pages` páginas a partir de `virt`, em frames novos e zerados. `bytes` começa `lead`
    /// bytes depois de `virt` (o `p_vaddr` de um segmento não é de página); o resto fica zero. A
    /// permissão é a do mapeador: W+X é recusado.
    pub fn map_at(
        &mut self,
        virt: u64,
        lead: usize,
        bytes: &[u8],
        pages: u64,
        flags: PageFlags,
    ) -> Result<(), MapError> {
        debug_assert!(virt >= USER_MIN && virt.is_multiple_of(PAGE_SIZE));
        let chunk = usize::try_from(PAGE_SIZE).expect("página cabe em usize");
        let data = lead..lead + bytes.len();
        for page in 0..pages {
            #[cfg(feature = "selftest")]
            if !spend_budget() {
                return Err(MapError::OutOfFrames);
            }
            let frame = self.frames.allocate_frame().ok_or(MapError::OutOfFrames)?;
            self.data_frames += 1;
            memory::zero_frame(frame);
            let first = usize::try_from(page).expect("índice cabe em usize") * chunk;
            let (from, to) = (data.start.max(first), data.end.min(first + chunk));
            if from < to {
                // SAFETY: `from..to` cabe na página e o frame acabou de sair do alocador, só nosso.
                unsafe { memory::fill_frame(frame, from - first, &bytes[from - lead..to - lead]) };
            }
            if flags.executable {
                memory::sync_code_frame(frame);
            }
            let at = VirtAddr::new(virt + page * PAGE_SIZE);
            let mut counted = Counted {
                frames: &mut self.frames,
                taken: 0,
            };
            let mapped = self.tables.map_page(at, frame, flags, &mut counted);
            self.table_frames += counted.taken;
            if let Err(error) = mapped {
                self.release(frame);
                return Err(error);
            }
        }
        Ok(())
    }

    /// Lê `out.len()` bytes do usuário em `virt` (dentro de uma página) pelo frame que a mapeia; `false`
    /// se a página não está mapeada. Só o modo de teste.
    #[cfg(feature = "selftest")]
    pub fn read(&self, virt: u64, out: &mut [u8]) -> bool {
        let page = virt & !(PAGE_SIZE - 1);
        let offset = usize::try_from(virt - page).unwrap_or(usize::MAX);
        let mut frame = None;
        self.tables.walk(&mut |mapping| {
            // No aarch64 a caminhada devolve o endereço canônico da metade alta (`TTBR1`).
            if mapping.virt.as_u64() & 0x0000_FFFF_FFFF_FFFF == page {
                frame = Some(mapping.phys);
            }
        });
        let Some(frame) = frame.filter(|_| offset.saturating_add(out.len()) <= 4096) else {
            return false;
        };
        // SAFETY: o frame é de dados deste espaço, vivo, e o intervalo cabe na página.
        unsafe { memory::read_frame(frame, offset, out) };
        true
    }

    /// Frames de dados e de tabelas (a raiz inclusive) que o espaço tem.
    pub fn stats(&self) -> SpaceStats {
        SpaceStats {
            data: self.data_frames,
            tables: self.table_frames,
        }
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
