//! O espaço de endereçamento de um processo de usuário: tabelas próprias para a metade baixa
//! (`CR3` ou `TTBR0_EL1`), frames que ele mapeia, a falta de página que cria a página no primeiro
//! toque, o *shootdown* de TLB e a devolução dos frames, zerados, na morte (SDD 17).

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
#[cfg(feature = "selftest")]
use rustos_hal::AddressSpace;
use rustos_hal::abi::{USER_MIN, USER_TOP};

use rustos_hal::{FrameSource, MapError, PAGE_SIZE, PageFlags, PageMapper, PhysAddr, VirtAddr};
use rustos_mm::PhysFrame;
use rustos_mm::vmar::{Access, FaultError, FaultPlan, Mapped};

use crate::frames::Frames;
use crate::loader::RegionTree;
use crate::platform::{self, PlatformSpace};
use crate::{commit, memory, tlb};

/// O frame da página zero compartilhada, ou 0 enquanto nenhum processo leu página nunca tocada. Nasce
/// no primeiro uso, zerado, e nunca volta ao alocador (SDD 17, "Página zero compartilhada e COW").
static ZERO_FRAME: AtomicU64 = AtomicU64::new(0);

/// A página zero, se já existe.
fn zero_page_now() -> Option<PhysAddr> {
    Some(ZERO_FRAME.load(Ordering::Acquire))
        .filter(|&frame| frame != 0)
        .map(PhysAddr::new)
}

/// A página zero; cria o frame na primeira chamada. Duas CPUs que a criam juntas ficam com uma só.
fn zero_page(frames: Frames) -> Option<PhysAddr> {
    if let Some(frame) = zero_page_now() {
        return Some(frame);
    }
    let fresh = frames.allocate()?;
    memory::zero_frame(fresh.start());
    match ZERO_FRAME.compare_exchange(
        0,
        fresh.start().as_u64(),
        Ordering::AcqRel,
        Ordering::Acquire,
    ) {
        Ok(_) => Some(fresh.start()),
        Err(winner) => {
            let _ = frames.deallocate(fresh);
            Some(PhysAddr::new(winner))
        }
    }
}

/// O frame da página zero, se existe; o autoteste confere que nenhum espaço o mapeia gravável.
#[cfg(feature = "selftest")]
pub fn zero_page_frame() -> Option<PhysAddr> {
    zero_page_now()
}

/// O que a falta resolveu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolved {
    /// Leitura de página nunca tocada: mapeou a página zero.
    ZeroPage,
    /// Escrita em página nunca tocada: frame novo e zerado.
    Fresh,
    /// Escrita sobre a página zero: frame novo no lugar dela.
    CopiedZero,
    /// A página já servia ao acesso.
    Retried,
}

/// Por que a falta não se resolve: o processo morre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unresolved {
    Region(FaultError),
    OutOfMemory,
}

/// Faltas resolvidas desde o último `reset_fault_counts`, por tipo (só o modo de teste).
#[cfg(feature = "selftest")]
static RESOLVED: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

/// Zera os contadores de falta resolvida.
#[cfg(feature = "selftest")]
pub fn reset_fault_counts() {
    for count in &RESOLVED {
        count.store(0, Ordering::Relaxed);
    }
}

/// Faltas resolvidas: página zero, frame novo, cópia da página zero, repetição.
#[cfg(feature = "selftest")]
pub fn fault_counts() -> [u64; 4] {
    core::array::from_fn(|index| RESOLVED[index].load(Ordering::Relaxed))
}

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
    /// Páginas preguiçosas prometidas e quantas já têm frame próprio.
    pub lazy_reserved: u64,
    pub lazy_backed: u64,
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
    /// CPUs que rodam o espaço agora, um bit por CPU; o endereço dela fica na tarefa (`bind_user`).
    active: AtomicU32,
    /// Páginas preguiçosas prometidas e as que já ganharam frame próprio.
    lazy_reserved: u64,
    lazy_backed: u64,
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
            active: AtomicU32::new(0),
            lazy_reserved: 0,
            lazy_backed: 0,
        })
    }

    /// O endereço da máscara de CPUs do espaço, para `task::bind_user`. O espaço não pode se mover
    /// enquanto alguma CPU o roda (`wait_inactive`).
    pub fn active_addr(&self) -> usize {
        core::ptr::from_ref(&self.active).expose_provenance()
    }

    /// CPUs que rodam o espaço agora.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "lido só pelo modo de teste")
    )]
    pub fn active_cpus(&self) -> u32 {
        self.active.load(Ordering::SeqCst)
    }

    /// Promete `pages` páginas preguiçosas contra os frames livres (SDD 17, decisão 9). A promessa
    /// volta na morte do espaço, no que não ganhou frame.
    pub fn reserve_lazy(&mut self, pages: u64) -> Result<(), MapError> {
        if !commit::reserve(pages, self.frames.free_frames() as u64) {
            return Err(MapError::OutOfFrames);
        }
        self.lazy_reserved += pages;
        Ok(())
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
            lazy_reserved: self.lazy_reserved,
            lazy_backed: self.lazy_backed,
        }
    }

    fn release(&mut self, frame: PhysAddr) {
        self.data_frames -= 1;
        memory::zero_frame(frame);
        let _ = self.frames.deallocate(PhysFrame::containing(frame));
    }

    /// Um frame zerado para uma página preguiçosa, contra a promessa dela.
    fn fresh_frame(&mut self) -> Result<PhysAddr, Unresolved> {
        let frame = self
            .frames
            .allocate_frame()
            .ok_or(Unresolved::OutOfMemory)?;
        memory::zero_frame(frame);
        self.data_frames += 1;
        Ok(frame)
    }

    /// Mapeia `frame` em `page`; se não cabe, devolve o frame.
    fn map_resolved(
        &mut self,
        page: VirtAddr,
        frame: PhysAddr,
        flags: PageFlags,
    ) -> Result<(), Unresolved> {
        let mut counted = Counted {
            frames: &mut self.frames,
            taken: 0,
        };
        let mapped = self.tables.map_page(page, frame, flags, &mut counted);
        self.table_frames += counted.taken;
        mapped.map_err(|_| Unresolved::OutOfMemory)
    }

    /// A página que ganhou frame próprio sai da promessa.
    fn backed(&mut self) {
        self.lazy_backed += 1;
        commit::release(1);
    }

    /// Resolve a falta de página de usuário em `address`: página zero na leitura, frame novo na
    /// escrita, cópia da página zero na escrita sobre ela. Só uma thread por espaço chama
    /// (DEBT-0120).
    pub fn resolve_fault(
        &mut self,
        tree: &RegionTree,
        address: u64,
        access: Access,
    ) -> Result<Resolved, Unresolved> {
        let page = VirtAddr::new(address & !(PAGE_SIZE - 1));
        let zero = zero_page_now();
        let mapped = match self.tables.leaf(page) {
            None => Mapped::Absent,
            Some((frame, _)) if Some(frame) == zero => Mapped::ZeroPage,
            Some(_) => Mapped::Private,
        };
        let plan = tree
            .plan_fault(address, access, mapped)
            .map_err(Unresolved::Region)?;
        let done = match plan {
            FaultPlan::MapZero => {
                let zero = zero_page(self.frames).ok_or(Unresolved::OutOfMemory)?;
                self.map_resolved(page, zero, PageFlags::USER_RODATA)?;
                Resolved::ZeroPage
            }
            FaultPlan::MapFresh => {
                let frame = self.fresh_frame()?;
                if let Err(error) = self.map_resolved(page, frame, PageFlags::USER_DATA) {
                    self.release(frame);
                    return Err(error);
                }
                self.backed();
                Resolved::Fresh
            }
            FaultPlan::CopyZero => {
                let frame = self.fresh_frame()?;
                if self
                    .tables
                    .remap_page(page, frame, PageFlags::USER_DATA)
                    .is_err()
                {
                    self.release(frame);
                    return Err(Unresolved::OutOfMemory);
                }
                self.flush_remote(page.as_u64());
                self.backed();
                Resolved::CopiedZero
            }
            FaultPlan::Retry => {
                self.tables.forget(page);
                Resolved::Retried
            }
        };
        #[cfg(feature = "selftest")]
        RESOLVED[done as usize].fetch_add(1, Ordering::Relaxed);
        Ok(done)
    }

    /// Invalida `virt` nas outras CPUs que rodam o espaço e espera todas: a entrada já mudou nesta.
    pub fn flush_remote(&self, virt: u64) {
        tlb::shoot_page(virt, self.active.load(Ordering::SeqCst));
    }

    /// Remove a página de `virt`, invalida o TLB de todas as CPUs que rodam o espaço e só então
    /// devolve o frame, zerado. A página zero nunca volta. `false` se não havia página. Quem remove
    /// uma região preguiçosa acerta a promessa dela.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(
            dead_code,
            reason = "o `vmar_unmap` da 4c a usa; hoje só o modo de teste"
        )
    )]
    pub fn unmap_page(&mut self, virt: u64) -> bool {
        let Some(frame) = self.tables.unmap_page(VirtAddr::new(virt)) else {
            return false;
        };
        self.flush_remote(virt);
        if Some(frame) != zero_page_now() {
            self.release(frame);
        }
        true
    }

    /// Invalida `virt` nesta CPU e nas outras do espaço sem mudar a tabela; mede o custo do
    /// *shootdown* no modo de teste.
    #[cfg(feature = "selftest")]
    pub fn flush_page(&self, virt: u64) {
        self.tables.forget(VirtAddr::new(virt));
        self.flush_remote(virt);
    }

    /// Nenhuma folha da metade de usuário mapeia a página zero gravável ou executável (o W^X do usuário).
    #[cfg(feature = "selftest")]
    pub fn zero_page_is_read_only_everywhere(&self) -> bool {
        let zero = zero_page_now();
        let mut clean = true;
        self.tables.walk(&mut |mapping| {
            if Some(mapping.phys) == zero && mapping.virt.as_u64() < USER_TOP {
                clean &= !mapping.flags.writable && !mapping.flags.executable && mapping.flags.user;
            }
        });
        clean
    }

    /// Escreve `bytes` no começo da página própria de `virt` (só o modo de teste).
    #[cfg(feature = "selftest")]
    pub fn write_private(&self, virt: u64, bytes: &[u8]) -> bool {
        let Some((frame, flags)) = self.tables.leaf(VirtAddr::new(virt & !(PAGE_SIZE - 1))) else {
            return false;
        };
        if !flags.writable || bytes.len() > 4096 {
            return false;
        }
        // SAFETY: o frame é próprio deste espaço (gravável), e o tamanho cabe na página.
        unsafe { memory::fill_frame(frame, 0, bytes) };
        true
    }

    /// Espera nenhuma CPU rodar o espaço: as tabelas só se liberam depois disso. Atende os
    /// *shootdowns* dos outros enquanto gira, para não travar quem espera por esta CPU. Chamar antes
    /// de mover o espaço: a máscara que as CPUs baixam é a do endereço em que ele foi ligado.
    pub fn wait_inactive(&self) {
        let deadline = platform::now_ns().map(|now| now + 5_000_000_000);
        let mut spins = 0_u64;
        while self.active.load(Ordering::SeqCst) != 0 {
            tlb::service();
            spins += 1;
            let late = deadline
                .zip(platform::now_ns())
                .is_some_and(|(limit, now)| now > limit);
            assert!(
                !late && spins < 2_000_000_000,
                "espaço de usuário ainda ativo em outra CPU"
            );
            core::hint::spin_loop();
        }
    }

    /// A metade do kernel da raiz é a da raiz do kernel.
    pub fn kernel_half_in_sync(&self) -> bool {
        platform::kernel_half_in_sync(&self.tables)
    }

    /// Devolve ao alocador todo frame de dado (menos a página zero), toda tabela e a raiz, zerados, e
    /// a promessa que sobrou. Espera nenhuma CPU rodar o espaço.
    pub fn destroy(mut self) {
        self.wait_inactive();
        let frames = self.frames;
        let zero = zero_page_now();
        self.tables.release_user_half(USER_TOP_ENTRIES, |frame| {
            if Some(frame) == zero {
                return;
            }
            memory::zero_frame(frame);
            let _ = frames.deallocate(PhysFrame::containing(frame));
        });
        commit::release(self.lazy_reserved - self.lazy_backed);
    }
}
