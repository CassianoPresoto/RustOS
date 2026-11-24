//! O VMO (objeto de memória virtual, SDD 17, fatia 4c): um conjunto de páginas que um ou mais espaços
//! mapeiam. O anônimo é preguiçoso e dono dos frames (zerados no primeiro toque, devolvidos zerados
//! quando o último `Arc` solta); o físico aponta frames que já existem (o initramfs), só leitura,
//! sem zerar e sem devolver. O de framebuffer é físico também, mas mapeia com write-combining (SDD 18).

use alloc::sync::Arc;
use alloc::vec::Vec;

use rustos_hal::abi::{PROCESS_BYTES_MAX, Status};
use rustos_hal::{CacheMode, PhysAddr};
use rustos_mm::PhysFrame;
use rustos_object::HandleCount;

use crate::frames::Frames;
use crate::object::{ByteCharge, Quota};
use crate::sync::IrqLock;
use crate::uspace::Unresolved;
use crate::{commit, memory};

const PAGE: u64 = rustos_hal::PAGE_SIZE;
/// Bytes que o objeto VMO custa ao criador, mais `SLOT_BYTES` por página: o índice de frames nasce
/// inteiro, para a falta de página nunca falhar por cota (só por frame).
pub const VMO_BYTES: usize = 256;
const SLOT_BYTES: usize = size_of::<u32>();
const _: () = assert!(VMO_BYTES >= size_of::<Vmo>());
const _: () = assert!(PROCESS_BYTES_MAX > VMO_BYTES);

#[derive(Debug)]
enum Kind {
    Anonymous,
    /// Frames contíguos que o kernel não aloca nem devolve.
    Physical(PhysAddr),
    /// Como `Physical`, do framebuffer: o mapeamento é write-combining e segura a tela (`framebuffer`).
    Framebuffer(PhysAddr),
}

/// Os frames que o VMO anônimo já tem: `slots[i]` é o número do frame da página `i` mais um, ou 0.
#[derive(Debug, Default)]
struct Store {
    slots: Vec<u32>,
    committed: u64,
}

#[derive(Debug)]
pub struct Vmo {
    handles: HandleCount,
    pages: u64,
    kind: Kind,
    frames: Frames,
    store: IrqLock<Store>,
    /// O objeto e o índice de frames, cobrados ao criador até o último `Arc` soltar.
    #[allow(dead_code, reason = "só o `Drop` da cobrança importa")]
    cost: Option<ByteCharge>,
}

impl Vmo {
    /// Um VMO anônimo de `pages` páginas, cobrado a `quota` e prometido contra os frames livres
    /// (decisão 9: `NoMemory` agora, e não a morte na falta de página).
    pub fn anonymous(frames: Frames, quota: &Arc<Quota>, pages: u64) -> Result<Arc<Self>, Status> {
        let count = usize::try_from(pages).map_err(|_| Status::InvalidArgs)?;
        let bytes = count
            .checked_mul(SLOT_BYTES)
            .and_then(|slots| slots.checked_add(VMO_BYTES))
            .ok_or(Status::NoMemory)?;
        let cost = ByteCharge::new(quota, bytes).ok_or(Status::NoMemory)?;
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(count)
            .map_err(|_| Status::NoMemory)?;
        slots.resize(count, 0);
        if !commit::reserve(pages, frames.free_frames() as u64) {
            return Err(Status::NoMemory);
        }
        Ok(Arc::new(Self {
            handles: HandleCount::new(0),
            pages,
            kind: Kind::Anonymous,
            frames,
            store: IrqLock::new(Store {
                slots,
                committed: 0,
            }),
            cost: Some(cost),
        }))
    }

    /// Um VMO físico sobre `pages` páginas a partir de `base` (o initramfs). Sem cobrança nem promessa.
    pub fn physical(frames: Frames, base: PhysAddr, pages: u64) -> Arc<Self> {
        Arc::new(Self {
            handles: HandleCount::new(0),
            pages,
            kind: Kind::Physical(base),
            frames,
            store: IrqLock::new(Store::default()),
            cost: None,
        })
    }

    /// Um VMO sobre a memória do framebuffer, `pages` páginas a partir de `base`. Como o físico, não
    /// cobra nem devolve frames; só o mapeamento o diferencia.
    pub fn framebuffer(frames: Frames, base: PhysAddr, pages: u64) -> Arc<Self> {
        Arc::new(Self {
            handles: HandleCount::new(0),
            pages,
            kind: Kind::Framebuffer(base),
            frames,
            store: IrqLock::new(Store::default()),
            cost: None,
        })
    }

    pub fn is_framebuffer(&self) -> bool {
        matches!(self.kind, Kind::Framebuffer(_))
    }

    /// A política de cache das páginas que o mapeamento dele cria.
    pub const fn cache(&self) -> CacheMode {
        match self.kind {
            Kind::Framebuffer(_) => CacheMode::WriteCombining,
            Kind::Anonymous | Kind::Physical(_) => CacheMode::WriteBack,
        }
    }

    pub const fn handles(&self) -> &HandleCount {
        &self.handles
    }

    pub const fn pages(&self) -> u64 {
        self.pages
    }

    /// O frame da página `page`, criando-o zerado se o VMO é anônimo e ela nunca foi tocada. A
    /// página que ganha frame troca a promessa por ele.
    pub fn frame_at(&self, page: u64) -> Result<PhysAddr, Unresolved> {
        if page >= self.pages {
            return Err(Unresolved::Region(rustos_mm::vmar::FaultError::Unbacked));
        }
        if let Kind::Physical(base) | Kind::Framebuffer(base) = self.kind {
            return Ok(PhysAddr::new(base.as_u64() + page * PAGE));
        }
        let index = usize::try_from(page).map_err(|_| Unresolved::OutOfMemory)?;
        let mut store = self.store.lock();
        if let Some(&slot) = store.slots.get(index)
            && slot != 0
        {
            return Ok(frame_of(slot));
        }
        let frame = self
            .frames
            .allocate()
            .ok_or(Unresolved::OutOfMemory)?
            .start();
        let Some(slot) = slot_of(frame) else {
            let _ = self.frames.deallocate(PhysFrame::containing(frame));
            return Err(Unresolved::OutOfMemory);
        };
        memory::zero_frame(frame);
        store.slots[index] = slot;
        store.committed += 1;
        commit::release(1);
        Ok(frame)
    }

    /// Páginas que já têm frame próprio (só o anônimo).
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "lido só pelo modo de teste")
    )]
    pub fn committed(&self) -> u64 {
        self.store.lock().committed
    }
}

/// O frame como valor de `slots`: o número dele mais um (0 é página sem frame).
fn slot_of(frame: PhysAddr) -> Option<u32> {
    u32::try_from(frame.as_u64() / PAGE + 1).ok()
}

fn frame_of(slot: u32) -> PhysAddr {
    PhysAddr::new((u64::from(slot) - 1) * PAGE)
}

impl Drop for Vmo {
    fn drop(&mut self) {
        let store = self.store.lock();
        for &slot in &store.slots {
            if slot != 0 {
                let frame = frame_of(slot);
                memory::zero_frame(frame);
                let _ = self.frames.deallocate(PhysFrame::containing(frame));
            }
        }
        if matches!(self.kind, Kind::Anonymous) {
            commit::release(self.pages - store.committed);
        }
    }
}
