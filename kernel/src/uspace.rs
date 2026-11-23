//! O espaço de endereçamento de um processo de usuário: tabelas próprias para a metade baixa
//! (`CR3` ou `TTBR0_EL1`), frames que ele mapeia, a falta de página que cria a página no primeiro
//! toque, o *shootdown* de TLB e a devolução dos frames, zerados, na morte (SDD 17).

use alloc::sync::Arc;
use core::num::NonZeroU32;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering, fence};

#[cfg(feature = "selftest")]
use rustos_hal::AddressSpace;
use rustos_hal::MAX_CPUS;
use rustos_hal::abi::{USER_MIN, USER_TOP};

use rustos_hal::{FrameSource, MapError, PAGE_SIZE, PageFlags, PageMapper, PhysAddr, VirtAddr};
use rustos_mm::PhysFrame;
use rustos_mm::vmar::{Access, FaultError, FaultPlan, Mapped, Perms};

use crate::frames::Frames;
use crate::loader::RegionTree;
use crate::platform::{self, PlatformSpace};
use crate::vmo::Vmo;
use crate::{commit, memory, tlb};

/// Quem é dono dos frames de um intervalo que sai do espaço.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pages {
    /// Frames de um VMO ou do initramfs: a folha sai, o frame fica.
    Borrowed,
    /// Frames do espaço (imagem, pilha, `.bss`): voltam ao alocador, zerados.
    Owned,
    /// Como `Owned`, de região preguiçosa: acerta também a promessa.
    OwnedLazy,
}

/// Páginas que um `unmap` invalida com um IPI só.
const UNMAP_CHUNK: usize = 32;

/// As permissões de uma região como flags de página: escrita ganha leitura (o x86 não tem só
/// escrita) e execução vem sem escrita (W^X já foi recusado na árvore).
const fn flags_of(perms: Perms) -> PageFlags {
    if perms.contains(Perms::EXECUTE) {
        PageFlags::USER_CODE
    } else if perms.contains(Perms::WRITE) {
        PageFlags::USER_DATA
    } else {
        PageFlags::USER_RODATA
    }
}

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

/// A página zero segue inteira em zero (ninguém escreveu nela por um mapa gravável nem pelo mapa
/// direto). Só o modo de teste.
#[cfg(feature = "selftest")]
pub fn zero_page_is_clean() -> bool {
    let Some(frame) = zero_page_now() else {
        return true;
    };
    let mut chunk = [0xFF_u8; 512];
    (0..usize::try_from(PAGE_SIZE).unwrap_or(4096))
        .step_by(512)
        .all(|offset| {
            // SAFETY: a página zero é um frame vivo, e o intervalo cabe nela.
            unsafe { memory::read_frame(frame, offset, &mut chunk) };
            chunk.iter().all(|&byte| byte == 0)
        })
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
    /// A página de um VMO: o frame é dele, só se mapeou.
    Object,
}

/// Por que a falta não se resolve: o processo morre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unresolved {
    Region(FaultError),
    OutOfMemory,
}

/// Faltas resolvidas desde o último `reset_fault_counts`, por tipo (só o modo de teste).
#[cfg(feature = "selftest")]
static RESOLVED: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];

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

/// O que as CPUs compartilham sobre o espaço, e o endereço dele fica na tarefa (`bind_user`): a
/// máscara de CPUs que o rodam agora e a etiqueta de TLB, com a regra de migração (SDD 17, "PCID e
/// ASID com várias CPUs").
#[derive(Debug)]
pub struct Shared {
    /// CPUs que rodam o espaço agora, um bit por CPU.
    pub active: AtomicU32,
    /// O `Asid` cru que o gerador deu ao espaço (zero: nenhum ainda).
    pub asid: AtomicU64,
    /// Sobe a cada `unmap`: a CPU que volta a rodar o espaço compara com `seen`.
    pub tlb_gen: AtomicU64,
    /// A `tlb_gen` que cada CPU viu ao carregar a etiqueta (ou esvaziou) para este espaço.
    pub seen: [AtomicU64; MAX_CPUS],
}

impl Shared {
    const fn new() -> Self {
        Self {
            active: AtomicU32::new(0),
            asid: AtomicU64::new(0),
            tlb_gen: AtomicU64::new(0),
            seen: [const { AtomicU64::new(0) }; MAX_CPUS],
        }
    }
}

#[derive(Debug)]
pub struct UserSpace {
    tables: PlatformSpace,
    frames: Frames,
    data_frames: u64,
    table_frames: u64,
    shared: Shared,
    /// Páginas preguiçosas prometidas, o que a promessa devolve ao morrer (as páginas mais as
    /// tabelas que elas pedem) e as páginas que já ganharam frame próprio.
    lazy_reserved: u64,
    lazy_promised: u64,
    lazy_backed: u64,
    /// Páginas de região preguiçosa que nasceram com frame (contadas como prometidas e com frame
    /// para que o `unmap` acerte cada página igual): fora do que `stats` diz.
    lazy_eager: u64,
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
            shared: Shared::new(),
            lazy_reserved: 0,
            lazy_promised: 0,
            lazy_backed: 0,
            lazy_eager: 0,
        })
    }

    /// O endereço da máscara de CPUs do espaço, para `task::bind_user`. O espaço não pode se mover
    /// enquanto alguma CPU o roda (`wait_inactive`).
    pub fn active_addr(&self) -> usize {
        core::ptr::from_ref(&self.shared).expose_provenance()
    }

    /// CPUs que rodam o espaço agora.
    pub fn active_cpus(&self) -> u32 {
        self.shared.active.load(Ordering::SeqCst)
    }

    /// Promete `pages` páginas preguiçosas contra os frames livres (SDD 17, decisão 9), mais as
    /// tabelas de página que elas podem pedir (uma por 512 páginas e mais uma). Chamar depois de
    /// mapear o que nasce com frame, para os livres refletirem o que já foi usado. A promessa volta
    /// na morte do espaço, no que não ganhou frame.
    pub fn reserve_lazy(&mut self, pages: u64) -> Result<(), MapError> {
        if pages == 0 {
            return Ok(());
        }
        let promised = pages + pages.div_ceil(512) + 1;
        if !commit::reserve(promised, self.frames.free_frames() as u64) {
            return Err(MapError::OutOfFrames);
        }
        self.lazy_reserved += pages;
        self.lazy_promised += promised;
        Ok(())
    }

    /// Conta `pages` páginas de região preguiçosa que já nasceram com frame (a página de arquivo do
    /// segmento com `.bss`, a primeira página da pilha): o `unmap` as trata como as outras.
    pub const fn count_eager(&mut self, pages: u64) {
        self.lazy_reserved += pages;
        self.lazy_promised += pages;
        self.lazy_backed += pages;
        self.lazy_eager += pages;
    }

    /// Promete `frames` frames para as tabelas de um mapeamento (decisão 9). A promessa volta em
    /// `release_promise`.
    pub fn reserve_promise(&self, frames: u64) -> Result<(), MapError> {
        if commit::reserve(frames, self.frames.free_frames() as u64) {
            Ok(())
        } else {
            Err(MapError::OutOfFrames)
        }
    }

    /// Devolve uma promessa de `reserve_promise`.
    #[allow(
        clippy::unused_self,
        reason = "a promessa é global; `self` prova que o espaço a fez"
    )]
    pub fn release_promise(&self, frames: u64) {
        commit::release(frames);
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
            lazy_reserved: self.lazy_reserved.saturating_sub(self.lazy_eager),
            lazy_backed: self.lazy_backed.saturating_sub(self.lazy_eager),
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
    /// escrita, cópia da página zero na escrita sobre ela. O chamador tem o lock do espaço.
    #[cfg(feature = "selftest")]
    pub fn resolve_fault(
        &mut self,
        tree: &RegionTree,
        address: u64,
        access: Access,
    ) -> Result<Resolved, Unresolved> {
        self.resolve_fault_with(tree, address, access, &|_| None)
    }

    /// Como `resolve_fault`; `objects` acha o VMO de uma região com `Backing`.
    pub fn resolve_fault_with<'a>(
        &mut self,
        tree: &RegionTree,
        address: u64,
        access: Access,
        objects: &dyn Fn(NonZeroU32) -> Option<&'a Arc<Vmo>>,
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
                // Troca uma folha existente (a página zero, lida numa CPU): a geração sobe antes, como em
                // `unmap_pages`, para a CPU que volta ao espaço esvaziar a etiqueta.
                self.shared.tlb_gen.fetch_add(1, Ordering::SeqCst);
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
            FaultPlan::MapObject => {
                let region = tree
                    .check_fault(address, access)
                    .map_err(Unresolved::Region)?;
                let (id, index) = region
                    .object_page(address)
                    .ok_or(Unresolved::Region(FaultError::Unbacked))?;
                let vmo = objects(id).ok_or(Unresolved::Region(FaultError::Unbacked))?;
                let frame = vmo.frame_at(index)?;
                let flags = flags_of(region.attrs.perms);
                self.map_resolved(page, frame, flags)?;
                if flags.executable {
                    memory::sync_code_frame(frame);
                }
                return Ok(Resolved::Object);
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
        fence(Ordering::SeqCst);
        tlb::shoot_page(virt, self.shared.active.load(Ordering::SeqCst));
    }

    /// Remove a página de `virt` de região `kind`; `false` se não havia página.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(
            dead_code,
            reason = "o `vmar_unmap` usa `unmap_pages`; esta só o modo de teste"
        )
    )]
    pub fn unmap_page(&mut self, virt: u64, kind: Pages) -> bool {
        let had = self.tables.leaf(VirtAddr::new(virt)).is_some();
        self.unmap_pages(virt, 1, kind);
        had
    }

    /// Remove as folhas de `pages` páginas a partir de `virt`, invalida o TLB de todas as CPUs
    /// que rodam o espaço (um IPI por 32 páginas) e só então devolve os frames que o espaço possui,
    /// zerados. A página zero nunca volta. Em região preguiçosa a promessa de cada página volta ao
    /// contador. O chamador tem o lock do espaço.
    pub fn unmap_pages(&mut self, virt: u64, pages: u64, kind: Pages) {
        // A geração sobe antes de qualquer entrada sair: a CPU que começa a rodar o espaço depois
        // da leitura da máscara lê a geração nova e esvazia a sua etiqueta.
        self.shared.tlb_gen.fetch_add(1, Ordering::SeqCst);
        let zero = zero_page_now();
        let mut done = 0;
        while done < pages {
            let count = (pages - done).min(UNMAP_CHUNK as u64);
            let start = virt + done * PAGE_SIZE;
            let mut removed = [None::<PhysAddr>; UNMAP_CHUNK];
            let mut any = false;
            for (offset, slot) in removed
                .iter_mut()
                .take(usize::try_from(count).unwrap_or(UNMAP_CHUNK))
                .enumerate()
            {
                *slot = self
                    .tables
                    .unmap_page(VirtAddr::new(start + offset as u64 * PAGE_SIZE));
                any |= slot.is_some();
            }
            if any {
                self.flush_remote_range(start, count);
            }
            for frame in removed
                .into_iter()
                .take(usize::try_from(count).unwrap_or(UNMAP_CHUNK))
            {
                self.settle(frame, zero, kind);
            }
            done += count;
        }
    }

    /// O destino de uma página que saiu: o frame volta se é do espaço e a promessa se acerta.
    fn settle(&mut self, frame: Option<PhysAddr>, zero: Option<PhysAddr>, kind: Pages) {
        let own = frame.filter(|&frame| Some(frame) != zero);
        match kind {
            Pages::Borrowed => {}
            Pages::Owned => {
                if let Some(frame) = own {
                    self.release(frame);
                }
            }
            Pages::OwnedLazy => {
                if let Some(frame) = own {
                    self.release(frame);
                    self.lazy_backed -= 1;
                } else {
                    commit::release(1);
                }
                self.lazy_promised -= 1;
                self.lazy_reserved -= 1;
            }
        }
    }

    /// Invalida `pages` páginas nas outras CPUs que rodam o espaço: as entradas já mudaram nesta.
    fn flush_remote_range(&self, virt: u64, pages: u64) {
        fence(Ordering::SeqCst);
        tlb::shoot_range(virt, pages, self.shared.active.load(Ordering::SeqCst));
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
        while self.shared.active.load(Ordering::SeqCst) != 0 {
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
    /// a promessa que sobrou. Nenhuma CPU pode rodar o espaço: quem o ligou espera `wait_inactive`
    /// antes de movê-lo para cá.
    pub fn destroy(mut self) {
        assert_eq!(self.active_cpus(), 0, "espaço de usuário destruído em uso");
        let frames = self.frames;
        let zero = zero_page_now();
        self.tables.release_user_half(USER_TOP_ENTRIES, |frame| {
            if Some(frame) == zero {
                return;
            }
            memory::zero_frame(frame);
            let _ = frames.deallocate(PhysFrame::containing(frame));
        });
        commit::release(self.lazy_promised - self.lazy_backed);
    }
}
