//! Árvore de regiões do espaço de usuário de um processo: o que está reservado, com que
//! permissão e a quem pertence o endereço que faltou. Pura: não toca tabela de página.

use core::num::NonZeroU32;

use rustos_hal::addr::{align_down, align_up};

use crate::FRAME_SIZE;

const PAGE: u64 = FRAME_SIZE;
/// Bits de página do sorteio da base (SDD 17): 2^28 páginas de 4 KiB, 1 TiB.
pub const ASLR_BITS: u32 = 28;
/// Rejeições de palavra viciada antes de usar a próxima sem conferir (9 palavras no máximo).
const REJECTION_TRIES: u32 = 8;

/// Permissões de uma região. W+X nunca existe: nenhuma construção a aceita.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Perms(u8);

impl Perms {
    /// Nenhuma permissão: a região reserva o endereço e toda falta nela é negada.
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1);
    pub const WRITE: Self = Self(2);
    pub const EXECUTE: Self = Self(4);

    /// As permissões de `self` mais as de `other`. Pode formar W+X, que o VMAR recusa.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// `self` tem todas as permissões de `other`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Escrita e execução juntas: o que o `reserve` e o `allocate` recusam.
    pub const fn is_write_execute(self) -> bool {
        self.contains(Self(Self::WRITE.0 | Self::EXECUTE.0))
    }

    /// O acesso `access` é permitido.
    pub const fn permits(self, access: Access) -> bool {
        match access {
            Access::Read => self.contains(Self::READ),
            Access::Write => self.contains(Self::WRITE),
            Access::Execute => self.contains(Self::EXECUTE),
        }
    }
}

/// O acesso que causou a falta de página.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    Execute,
}

/// De onde a região veio; a falta de página e o relatório de morte usam.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Segmento da imagem do executável.
    Image,
    /// Pilha: a falta na guarda é estouro de pilha.
    Stack,
    /// Mapeamento pedido pelo processo.
    Mapping,
}

/// O objeto que sustenta a região e onde ela começa dentro dele. Sem ele, a região é
/// anônima: a falta resolve com a página zero ou com um frame novo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backing {
    /// Identificador do objeto; o significado é de quem guarda a tabela de objetos.
    pub object: NonZeroU32,
    /// Página do objeto em que o início da região cai.
    pub page_offset: u32,
}

/// Tudo o que uma região guarda além da faixa. Uma guarda é uma página reservada, sem
/// mapa, logo abaixo ou acima: nada mais é alocado nela e a falta nela nunca resolve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attrs {
    pub perms: Perms,
    pub kind: Kind,
    /// Reserva a página logo abaixo do início, sem mapa.
    pub guard_below: bool,
    /// Reserva a página logo acima do fim, sem mapa.
    pub guard_above: bool,
    /// `None`: região anônima.
    pub backing: Option<Backing>,
    /// Anônima e preguiçosa: a reserva não traz frames, e a falta numa página ausente resolve com a
    /// página zero (leitura) ou um frame novo (escrita), conforme `Vmar::plan_fault`.
    pub lazy: bool,
}

impl Attrs {
    /// Região anônima, sem guardas.
    pub const fn new(kind: Kind, perms: Perms) -> Self {
        Self {
            perms,
            kind,
            guard_below: false,
            guard_above: false,
            backing: None,
            lazy: false,
        }
    }

    /// `self` preguiçosa: sem frames até o primeiro toque.
    #[must_use]
    pub const fn lazy(self) -> Self {
        Self { lazy: true, ..self }
    }

    /// `self` com as guardas de baixo e de cima ligadas ou desligadas.
    #[must_use]
    pub const fn with_guards(self, below: bool, above: bool) -> Self {
        Self {
            guard_below: below,
            guard_above: above,
            ..self
        }
    }

    /// `self` sustentada pelo objeto de `backing`.
    #[must_use]
    pub const fn with_backing(self, backing: Backing) -> Self {
        Self {
            backing: Some(backing),
            ..self
        }
    }

    const fn below(self) -> u64 {
        if self.guard_below { PAGE } else { 0 }
    }

    const fn above(self) -> u64 {
        if self.guard_above { PAGE } else { 0 }
    }
}

/// Uma região mapeável `[start, end)`; as guardas ficam fora dela, em `lo()` e `hi()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    /// Primeiro endereço mapeável, alinhado à página.
    pub start: u64,
    /// Fim exclusivo da parte mapeável.
    pub end: u64,
    pub attrs: Attrs,
}

const _: () = assert!(size_of::<Region>() == 32);

impl Region {
    /// Para preencher o armazenamento antes do primeiro `reserve`.
    pub const EMPTY: Self = Self {
        start: 0,
        end: 0,
        attrs: Attrs::new(Kind::Mapping, Perms::NONE),
    };

    /// Início do que a região reserva, com a guarda de baixo.
    pub const fn lo(&self) -> u64 {
        self.start - self.attrs.below()
    }

    /// Fim exclusivo do que a região reserva, com a guarda de cima.
    pub const fn hi(&self) -> u64 {
        self.end + self.attrs.above()
    }

    /// Páginas da parte mapeável.
    pub const fn pages(&self) -> u64 {
        (self.end - self.start) / PAGE
    }

    /// Página do objeto que cobre `addr`, se a região tem objeto e `addr` está nela.
    pub fn object_page(&self, addr: u64) -> Option<(NonZeroU32, u64)> {
        let backing = self.attrs.backing?;
        (self.start <= addr && addr < self.end).then(|| {
            let page = (addr - self.start) / PAGE;
            (backing.object, u64::from(backing.page_offset) + page)
        })
    }
}

/// O que há num endereço.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit<'a> {
    /// Endereço na parte mapeável.
    Region(&'a Region),
    /// Página de guarda da região.
    Guard(&'a Region),
    /// Livre: fora de qualquer região e de qualquer guarda.
    Hole,
}

/// Por que a falta de página não resolve: o processo morre.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultError {
    /// Fora de qualquer região.
    NoRegion,
    /// Na página de guarda (estouro de pilha, no caso da pilha).
    Guard,
    /// A região existe e não permite este acesso.
    Permission,
    /// A página não está mapeada e a região não é preguiçosa: nada a criar (o carregador mapeia
    /// tudo antes de rodar).
    Unbacked,
}

/// O que as tabelas dizem da página que faltou, para `Vmar::plan_fault`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mapped {
    /// Sem folha.
    Absent,
    /// A folha é a página zero compartilhada, só leitura.
    ZeroPage,
    /// A folha é um frame do processo.
    Private,
}

/// O que o tratador da falta faz numa região preguiçosa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultPlan {
    /// Mapeia a página zero, só leitura e sem execução.
    MapZero,
    /// Aloca um frame zerado e o mapeia gravável.
    MapFresh,
    /// Aloca um frame zerado e o põe no lugar da página zero, gravável; invalida a entrada.
    CopyZero,
    /// A página já serve ao acesso (outra CPU a resolveu antes, ou o TLB estava velho): só repete.
    Retry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmarError {
    /// Endereço ou tamanho que não é múltiplo de página.
    Misaligned,
    /// Tamanho zero, ou faixa do VMAR vazia.
    Empty,
    /// Fora da faixa do VMAR, ou `base + len` estoura.
    OutOfRange,
    /// A faixa, com as guardas, toca outra região.
    Overlap,
    /// O armazenamento não tem mais lugar para uma região.
    Full,
    /// Nenhuma faixa livre serve ao pedido.
    NoSpace,
    /// Permissões W+X.
    WriteExecute,
    /// Alinhamento que não é potência de dois, ou menor que uma página.
    BadAlignment,
}

/// Qual das posições que servem `allocate` escolhe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// A faixa mais baixa que serve: determinística, sem entropia.
    Lowest,
    /// Uniforme entre todas as posições que servem.
    Random,
}

/// O pedido de `allocate`: tamanho e alinhamento em bytes e a janela onde a faixa
/// reservada (guardas incluídas) tem de caber.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fit {
    /// Tamanho da parte mapeável, múltiplo de página.
    pub len: u64,
    /// Potência de dois, no mínimo uma página.
    pub align: u64,
    /// `[início, fim)` onde a faixa reservada tem de caber; o VMAR a corta nos seus limites.
    pub window: (u64, u64),
    pub placement: Placement,
}

/// A árvore: um vetor ordenado em armazenamento dado de fora (`[Region; N]` ou um
/// `Box<[Region]>`). Busca em O(log N); inserir e remover movem até N regiões de 32 bytes.
/// Capacidade fixa e `memmove` O(N): DEBT-0075.
#[derive(Debug)]
pub struct Vmar<S> {
    storage: S,
    count: usize,
    lo: u64,
    hi: u64,
}

impl<S: AsRef<[Region]> + AsMut<[Region]>> Vmar<S> {
    /// `[lo, hi)` é a faixa que o VMAR administra (`USER_MIN`, `USER_TOP`).
    pub fn new(storage: S, lo: u64, hi: u64) -> Result<Self, VmarError> {
        if !lo.is_multiple_of(PAGE) || !hi.is_multiple_of(PAGE) {
            return Err(VmarError::Misaligned);
        }
        if lo >= hi {
            return Err(VmarError::Empty);
        }
        Ok(Self {
            storage,
            count: 0,
            lo,
            hi,
        })
    }

    pub const fn range(&self) -> (u64, u64) {
        (self.lo, self.hi)
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn capacity(&self) -> usize {
        self.storage.as_ref().len()
    }

    /// As regiões em ordem crescente de endereço.
    pub fn regions(&self) -> &[Region] {
        &self.storage.as_ref()[..self.count]
    }

    /// Esvazia a árvore (morte do processo); o armazenamento fica para reúso.
    pub fn clear(&mut self) {
        self.count = 0;
    }

    /// Reserva `[start, start + len)` com `attrs`; recusa o que sobrepõe outra região,
    /// guardas inclusive.
    pub fn reserve(&mut self, start: u64, len: u64, attrs: Attrs) -> Result<(), VmarError> {
        check_args(len, attrs)?;
        if !start.is_multiple_of(PAGE) {
            return Err(VmarError::Misaligned);
        }
        let end = start.checked_add(len).ok_or(VmarError::OutOfRange)?;
        let lo = start
            .checked_sub(attrs.below())
            .ok_or(VmarError::OutOfRange)?;
        let hi = end
            .checked_add(attrs.above())
            .ok_or(VmarError::OutOfRange)?;
        if lo < self.lo || hi > self.hi {
            return Err(VmarError::OutOfRange);
        }
        let index = self.regions().partition_point(|region| region.lo() < lo);
        let regions = self.regions();
        let clear_below = index == 0 || regions[index - 1].hi() <= lo;
        let clear_above = index == regions.len() || regions[index].lo() >= hi;
        if !(clear_below && clear_above) {
            return Err(VmarError::Overlap);
        }
        self.insert(index, Region { start, end, attrs })
    }

    /// Escolhe e reserva uma faixa livre; devolve o início da parte mapeável. `rng` só é
    /// chamado com `Placement::Random`.
    pub fn allocate(
        &mut self,
        fit: Fit,
        attrs: Attrs,
        rng: &mut impl FnMut() -> u64,
    ) -> Result<u64, VmarError> {
        check_args(fit.len, attrs)?;
        if !fit.align.is_power_of_two() || fit.align < PAGE {
            return Err(VmarError::BadAlignment);
        }
        if self.count == self.capacity() {
            return Err(VmarError::Full);
        }
        let (index, start) = match fit.placement {
            Placement::Lowest => (0..=self.count).find_map(|gap| {
                self.positions(gap, &fit, attrs)
                    .map(|(first, _)| (gap, first))
            }),
            Placement::Random => self.pick_random(&fit, attrs, rng),
        }
        .ok_or(VmarError::NoSpace)?;
        let end = start + fit.len;
        self.insert(index, Region { start, end, attrs })?;
        Ok(start)
    }

    /// Solta `[start, start + len)`: remove as regiões inteiras e apara ou parte as que
    /// o intervalo só cobre em parte. A guarda de uma ponta aparada deixa de existir.
    pub fn unmap(&mut self, start: u64, len: u64) -> Result<(), VmarError> {
        if len == 0 {
            return Err(VmarError::Empty);
        }
        if !start.is_multiple_of(PAGE) || !len.is_multiple_of(PAGE) {
            return Err(VmarError::Misaligned);
        }
        let end = start.checked_add(len).ok_or(VmarError::OutOfRange)?;
        if start < self.lo || end > self.hi {
            return Err(VmarError::OutOfRange);
        }
        let first = self.regions().partition_point(|region| region.end <= start);
        let past = self.regions().partition_point(|region| region.start < end);
        if first >= past {
            return Ok(());
        }
        let whole = self.regions()[first];
        if whole.start < start && end < whole.end {
            if self.count == self.capacity() {
                return Err(VmarError::Full);
            }
            let (low, high) = split(whole, start, end);
            self.storage.as_mut()[first] = low;
            return self.insert(first + 1, high);
        }
        let regions = self.storage.as_mut();
        let mut kept = first;
        for index in first..past {
            let region = trim(regions[index], start, end);
            if let Some(region) = region {
                regions[kept] = region;
                kept += 1;
            }
        }
        regions.copy_within(past..self.count, kept);
        self.count -= past - kept;
        Ok(())
    }

    /// O que há em `addr`.
    pub fn find(&self, addr: u64) -> Hit<'_> {
        let regions = self.regions();
        let index = regions.partition_point(|region| region.lo() <= addr);
        let Some(region) = index.checked_sub(1).map(|i| &regions[i]) else {
            return Hit::Hole;
        };
        if addr >= region.hi() {
            Hit::Hole
        } else if addr < region.start || addr >= region.end {
            Hit::Guard(region)
        } else {
            Hit::Region(region)
        }
    }

    /// A decisão da falta de página: a região que resolve, ou por que o processo morre.
    pub fn check_fault(&self, addr: u64, access: Access) -> Result<&Region, FaultError> {
        match self.find(addr) {
            Hit::Hole => Err(FaultError::NoRegion),
            Hit::Guard(_) => Err(FaultError::Guard),
            Hit::Region(region) if region.attrs.perms.permits(access) => Ok(region),
            Hit::Region(_) => Err(FaultError::Permission),
        }
    }

    /// Decide a falta de página de usuário: o que o tratador faz, ou por que o processo morre.
    /// `mapped` é o que as tabelas dizem da página que faltou.
    pub fn plan_fault(
        &self,
        addr: u64,
        access: Access,
        mapped: Mapped,
    ) -> Result<FaultPlan, FaultError> {
        let region = self.check_fault(addr, access)?;
        match (mapped, access) {
            (Mapped::Private, _) | (Mapped::ZeroPage, Access::Read) => Ok(FaultPlan::Retry),
            _ if !region.attrs.lazy => Err(FaultError::Unbacked),
            (_, Access::Execute) => Err(FaultError::Permission),
            (Mapped::Absent, Access::Read) => Ok(FaultPlan::MapZero),
            (Mapped::Absent, Access::Write) => Ok(FaultPlan::MapFresh),
            (Mapped::ZeroPage, _) => Ok(FaultPlan::CopyZero),
        }
    }

    fn insert(&mut self, index: usize, region: Region) -> Result<(), VmarError> {
        if self.count == self.capacity() {
            return Err(VmarError::Full);
        }
        let regions = self.storage.as_mut();
        regions.copy_within(index..self.count, index + 1);
        regions[index] = region;
        self.count += 1;
        Ok(())
    }

    /// A faixa livre entre a região `gap - 1` e a `gap`, pelas pontas reservadas.
    fn gap(&self, gap: usize) -> (u64, u64) {
        let regions = self.regions();
        let from = if gap == 0 {
            self.lo
        } else {
            regions[gap - 1].hi()
        };
        let to = regions.get(gap).map_or(self.hi, Region::lo);
        (from, to)
    }

    /// Primeira posição e quantas posições alinhadas cabem na faixa livre `gap`.
    fn positions(&self, gap: usize, fit: &Fit, attrs: Attrs) -> Option<(u64, u64)> {
        let (from, to) = self.gap(gap);
        let from = from.max(fit.window.0);
        let to = to.min(fit.window.1);
        let first = align_up(from.checked_add(attrs.below())?, fit.align)?;
        let reach = to.checked_sub(fit.len.checked_add(attrs.above())?)?;
        let last = align_down(reach, fit.align);
        (first <= last).then(|| (first, (last - first) / fit.align + 1))
    }

    fn pick_random(
        &self,
        fit: &Fit,
        attrs: Attrs,
        rng: &mut impl FnMut() -> u64,
    ) -> Option<(usize, u64)> {
        let total: u64 = (0..=self.count)
            .filter_map(|gap| self.positions(gap, fit, attrs))
            .map(|(_, count)| count)
            .sum();
        if total == 0 {
            return None;
        }
        let mut pick = uniform_below(total, rng);
        (0..=self.count).find_map(|gap| {
            let (first, count) = self.positions(gap, fit, attrs)?;
            if pick < count {
                return Some((gap, first + pick * fit.align));
            }
            pick -= count;
            None
        })
    }
}

fn check_args(len: u64, attrs: Attrs) -> Result<(), VmarError> {
    if len == 0 {
        return Err(VmarError::Empty);
    }
    if !len.is_multiple_of(PAGE) {
        return Err(VmarError::Misaligned);
    }
    if attrs.perms.is_write_execute() {
        return Err(VmarError::WriteExecute);
    }
    if let Some(backing) = attrs.backing {
        let last = u64::from(backing.page_offset) + len / PAGE;
        if last > u64::from(u32::MAX) {
            return Err(VmarError::OutOfRange);
        }
    }
    Ok(())
}

/// O pedaço de `region` que sobra fora de `[start, end)`, ou `None` se nada sobra.
fn trim(region: Region, start: u64, end: u64) -> Option<Region> {
    if end <= region.start || region.end <= start {
        return Some(region);
    }
    if start <= region.start && region.end <= end {
        return None;
    }
    let mut kept = region;
    if region.start < start {
        kept.end = start;
        kept.attrs.guard_above = false;
    } else {
        kept = cut_front(region, end);
    }
    Some(kept)
}

/// Parte `region` em dois, tirando `[start, end)` do meio.
fn split(region: Region, start: u64, end: u64) -> (Region, Region) {
    let mut low = region;
    low.end = start;
    low.attrs.guard_above = false;
    (low, cut_front(region, end))
}

/// A cauda de `region` a partir de `from`, sem a guarda de baixo.
fn cut_front(region: Region, from: u64) -> Region {
    let mut tail = region;
    tail.start = from;
    tail.attrs.guard_below = false;
    if let Some(backing) = &mut tail.attrs.backing {
        let skipped = u32::try_from((from - region.start) / PAGE).unwrap_or(u32::MAX);
        debug_assert!(backing.page_offset.checked_add(skipped).is_some());
        backing.page_offset = backing.page_offset.saturating_add(skipped);
    }
    tail
}

/// Valor uniforme em `[0, bound)`: multiplicação de Lemire com rejeição, no máximo
/// `REJECTION_TRIES + 1` palavras. `bound` 0 ou 1 devolve 0, sem consumir palavra.
fn uniform_below(bound: u64, rng: &mut impl FnMut() -> u64) -> u64 {
    if bound <= 1 {
        return 0;
    }
    let threshold = bound.wrapping_neg() % bound;
    let mut word = rng();
    for _ in 0..REJECTION_TRIES {
        let wide = u128::from(word) * u128::from(bound);
        if low_half(wide) >= threshold {
            break;
        }
        word = rng();
    }
    high_half(u128::from(word) * u128::from(bound))
}

#[allow(clippy::cast_possible_truncation, reason = "metade baixa de propósito")]
const fn low_half(wide: u128) -> u64 {
    wide as u64
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "a metade alta cabe: bound < 2^64"
)]
const fn high_half(wide: u128) -> u64 {
    (wide >> 64) as u64
}

/// Quantas posições de página o sorteio de `aslr_base` tem: `2^ASLR_BITS`, ou menos se
/// `[floor, limit)` for mais curto; 0 se o intervalo não tem uma página inteira.
fn aslr_pages(floor: u64, limit: u64) -> u64 {
    if limit <= floor {
        return 0;
    }
    ((limit - floor) / PAGE).min(1 << ASLR_BITS)
}

/// Base sorteada para uma região do espaço (o `mmap_base`, o topo da pilha): uniforme
/// entre as `2^ASLR_BITS` páginas a partir de `floor`, cortada em `limit` (exclusivo).
/// `None` se `floor` não é de página ou o intervalo não tem uma página inteira.
pub fn aslr_base(floor: u64, limit: u64, rng: &mut impl FnMut() -> u64) -> Option<u64> {
    let pages = aslr_pages(floor, limit);
    if !floor.is_multiple_of(PAGE) || pages == 0 {
        return None;
    }
    Some(floor + uniform_below(pages, rng) * PAGE)
}

/// Bits de entropia que `aslr_base(floor, limit, ..)` entrega (`log2` das posições,
/// arredondado para baixo): `ASLR_BITS` com o intervalo cheio, menos com `limit` perto
/// de `floor`. O boot relata este valor; 0 se `aslr_base` devolve `None`.
pub fn aslr_bits(floor: u64, limit: u64) -> u32 {
    let pages = aslr_pages(floor, limit);
    if !floor.is_multiple_of(PAGE) || pages == 0 {
        return 0;
    }
    pages.ilog2()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::XorShift;

    const LO: u64 = 0x1_0000;
    const HI: u64 = 0x7FFF_FFFF_F000;
    const RW: Attrs = Attrs::new(Kind::Mapping, Perms::READ.union(Perms::WRITE));

    fn vmar<const N: usize>() -> Vmar<[Region; N]> {
        Vmar::new([Region::EMPTY; N], LO, HI).unwrap()
    }

    fn lowest(len: u64) -> Fit {
        Fit {
            len,
            align: PAGE,
            window: (0, u64::MAX),
            placement: Placement::Lowest,
        }
    }

    fn no_entropy() -> impl FnMut() -> u64 {
        || panic!("Lowest não sorteia")
    }

    #[test]
    fn region_costs_32_bytes() {
        assert_eq!(size_of::<Region>(), 32);
        assert_eq!(size_of::<Option<Backing>>(), 8);
    }

    #[test]
    fn new_checks_the_range() {
        let store = [Region::EMPTY; 1];
        assert_eq!(Vmar::new(store, 1, HI).unwrap_err(), VmarError::Misaligned);
        assert_eq!(Vmar::new(store, HI, HI).unwrap_err(), VmarError::Empty);
        assert_eq!(Vmar::new(store, HI, LO).unwrap_err(), VmarError::Empty);
    }

    #[test]
    fn reserve_refuses_bad_arguments() {
        let mut vmar = vmar::<4>();
        assert_eq!(vmar.reserve(LO, 0, RW), Err(VmarError::Empty));
        assert_eq!(vmar.reserve(LO + 1, PAGE, RW), Err(VmarError::Misaligned));
        assert_eq!(vmar.reserve(LO, PAGE + 1, RW), Err(VmarError::Misaligned));
        assert_eq!(
            vmar.reserve(LO - PAGE, PAGE, RW),
            Err(VmarError::OutOfRange)
        );
        assert_eq!(vmar.reserve(HI, PAGE, RW), Err(VmarError::OutOfRange));
        assert_eq!(
            vmar.reserve(HI - PAGE, 2 * PAGE, RW),
            Err(VmarError::OutOfRange)
        );
        let wx = Attrs::new(Kind::Image, Perms::WRITE.union(Perms::EXECUTE));
        assert_eq!(vmar.reserve(LO, PAGE, wx), Err(VmarError::WriteExecute));
        assert!(vmar.is_empty());
    }

    #[test]
    fn reserve_refuses_overlap_but_accepts_neighbours() {
        let mut vmar = vmar::<8>();
        vmar.reserve(LO + 4 * PAGE, 2 * PAGE, RW).unwrap();
        for (start, len) in [(4, 1), (5, 1), (3, 2), (5, 2), (3, 4), (4, 2), (0, 100)] {
            let got = vmar.reserve(LO + start * PAGE, len * PAGE, RW);
            assert_eq!(got, Err(VmarError::Overlap), "{start} {len}");
        }
        vmar.reserve(LO + 3 * PAGE, PAGE, RW).unwrap();
        vmar.reserve(LO + 6 * PAGE, PAGE, RW).unwrap();
        assert_eq!(vmar.len(), 3);
        let starts: Vec<u64> = vmar.regions().iter().map(|r| r.start).collect();
        assert_eq!(starts, [LO + 3 * PAGE, LO + 4 * PAGE, LO + 6 * PAGE]);
    }

    #[test]
    fn reserve_reports_full_after_n_regions() {
        let mut vmar = vmar::<2>();
        vmar.reserve(LO, PAGE, RW).unwrap();
        vmar.reserve(LO + 2 * PAGE, PAGE, RW).unwrap();
        assert_eq!(vmar.reserve(LO + 4 * PAGE, PAGE, RW), Err(VmarError::Full));
        let fit = lowest(PAGE);
        assert_eq!(
            vmar.allocate(fit, RW, &mut no_entropy()),
            Err(VmarError::Full)
        );
    }

    #[test]
    fn a_region_can_end_exactly_at_the_top_and_start_at_the_bottom() {
        let mut vmar = vmar::<4>();
        vmar.reserve(HI - PAGE, PAGE, RW).unwrap();
        vmar.reserve(LO, PAGE, RW).unwrap();
        assert_eq!(vmar.find(HI - 1), Hit::Region(&vmar.regions()[1]));
        assert_eq!(vmar.find(HI), Hit::Hole);
    }

    #[test]
    fn range_at_the_very_top_of_the_address_space_does_not_overflow() {
        let top = 0xFFFF_FFFF_FFFF_F000;
        let mut vmar = Vmar::new([Region::EMPTY; 4], top - 0x10_0000, top).unwrap();
        assert_eq!(vmar.reserve(top, PAGE, RW), Err(VmarError::OutOfRange));
        assert_eq!(vmar.reserve(top, 2 * PAGE, RW), Err(VmarError::OutOfRange));
        assert_eq!(
            vmar.reserve(top - PAGE, top, RW),
            Err(VmarError::OutOfRange)
        );
        assert_eq!(
            vmar.reserve(top - PAGE, 2 * PAGE, RW),
            Err(VmarError::OutOfRange)
        );
        let guarded = RW.with_guards(false, true);
        assert_eq!(
            vmar.reserve(top - PAGE, PAGE, guarded),
            Err(VmarError::OutOfRange)
        );
        vmar.reserve(top - PAGE, PAGE, RW).unwrap();
        let got = vmar.allocate(lowest(PAGE), RW, &mut no_entropy());
        assert_eq!(got, Ok(top - 0x10_0000));
        let wrap = !(PAGE - 1);
        assert_eq!(vmar.unmap(top - PAGE, wrap), Err(VmarError::OutOfRange));
        assert_eq!(vmar.find(u64::MAX), Hit::Hole);
        vmar.unmap(top - PAGE, PAGE).unwrap();
        assert_eq!(vmar.len(), 1);
    }

    #[test]
    fn allocation_that_would_overflow_base_plus_len_finds_no_space() {
        let top = 0xFFFF_FFFF_FFFF_F000;
        let mut vmar = Vmar::new([Region::EMPTY; 4], top - 4 * PAGE, top).unwrap();
        let huge = Fit {
            len: top - PAGE,
            align: 1 << 62,
            window: (0, u64::MAX),
            placement: Placement::Lowest,
        };
        assert_eq!(
            vmar.allocate(huge, RW, &mut no_entropy()),
            Err(VmarError::NoSpace)
        );
        let aligned_past_end = Fit {
            len: PAGE,
            align: 1 << 63,
            window: (0, u64::MAX),
            placement: Placement::Lowest,
        };
        assert_eq!(
            vmar.allocate(aligned_past_end, RW, &mut no_entropy()),
            Err(VmarError::NoSpace)
        );
    }

    #[test]
    fn allocate_checks_alignment_and_size() {
        let mut vmar = vmar::<4>();
        let mut fit = lowest(PAGE);
        for align in [0, 1, PAGE / 2, 3 * PAGE] {
            fit.align = align;
            assert_eq!(
                vmar.allocate(fit, RW, &mut no_entropy()),
                Err(VmarError::BadAlignment)
            );
        }
        assert_eq!(
            vmar.allocate(lowest(0), RW, &mut no_entropy()),
            Err(VmarError::Empty)
        );
        assert_eq!(
            vmar.allocate(lowest(PAGE + 1), RW, &mut no_entropy()),
            Err(VmarError::Misaligned)
        );
        let wx = Attrs::new(Kind::Mapping, Perms::WRITE.union(Perms::EXECUTE));
        assert_eq!(
            vmar.allocate(lowest(PAGE), wx, &mut no_entropy()),
            Err(VmarError::WriteExecute)
        );
    }

    #[test]
    fn lowest_takes_the_first_hole_that_fits_and_honours_alignment() {
        let mut vmar = vmar::<8>();
        vmar.reserve(LO, PAGE, RW).unwrap();
        vmar.reserve(LO + 3 * PAGE, PAGE, RW).unwrap();
        let mut no = no_entropy();
        assert_eq!(vmar.allocate(lowest(2 * PAGE), RW, &mut no), Ok(LO + PAGE));
        assert_eq!(vmar.allocate(lowest(PAGE), RW, &mut no), Ok(LO + 4 * PAGE));
        let aligned = Fit {
            align: 1 << 30,
            ..lowest(PAGE)
        };
        assert_eq!(vmar.allocate(aligned, RW, &mut no), Ok(1 << 30));
        let again = vmar.allocate(aligned, RW, &mut no).unwrap();
        assert_eq!(again, 2 << 30);
    }

    #[test]
    fn window_bounds_the_reserved_extent() {
        let mut vmar = vmar::<8>();
        let fit = Fit {
            window: (LO + 10 * PAGE, LO + 12 * PAGE),
            ..lowest(2 * PAGE)
        };
        assert_eq!(
            vmar.allocate(fit, RW, &mut no_entropy()),
            Ok(LO + 10 * PAGE)
        );
        assert_eq!(
            vmar.allocate(fit, RW, &mut no_entropy()),
            Err(VmarError::NoSpace)
        );
        let guarded = RW.with_guards(true, true);
        let fit = Fit {
            window: (LO + 20 * PAGE, LO + 24 * PAGE),
            ..lowest(2 * PAGE)
        };
        let got = vmar.allocate(fit, guarded, &mut no_entropy()).unwrap();
        assert_eq!(got, LO + 21 * PAGE);
        let tight = Fit {
            window: (LO + 30 * PAGE, LO + 33 * PAGE),
            ..lowest(2 * PAGE)
        };
        assert_eq!(
            vmar.allocate(tight, guarded, &mut no_entropy()),
            Err(VmarError::NoSpace)
        );
    }

    #[test]
    fn full_space_has_no_room() {
        let mut vmar = Vmar::new([Region::EMPTY; 4], LO, LO + 4 * PAGE).unwrap();
        vmar.reserve(LO, 4 * PAGE, RW).unwrap();
        assert_eq!(
            vmar.allocate(lowest(PAGE), RW, &mut no_entropy()),
            Err(VmarError::NoSpace)
        );
        let mut rng = || 0;
        let random = Fit {
            placement: Placement::Random,
            ..lowest(PAGE)
        };
        assert_eq!(vmar.allocate(random, RW, &mut rng), Err(VmarError::NoSpace));
    }

    #[test]
    fn fragmentation_blocks_a_big_request_until_neighbours_are_released() {
        let mut vmar = Vmar::new([Region::EMPTY; 16], LO, LO + 8 * PAGE).unwrap();
        for page in (0..8).step_by(2) {
            vmar.reserve(LO + page * PAGE, PAGE, RW).unwrap();
        }
        let mut no = no_entropy();
        assert_eq!(
            vmar.allocate(lowest(2 * PAGE), RW, &mut no),
            Err(VmarError::NoSpace)
        );
        vmar.unmap(LO + 2 * PAGE, PAGE).unwrap();
        assert_eq!(vmar.allocate(lowest(3 * PAGE), RW, &mut no), Ok(LO + PAGE));
        assert_eq!(vmar.allocate(lowest(PAGE), RW, &mut no), Ok(LO + 5 * PAGE));
        assert_eq!(vmar.allocate(lowest(PAGE), RW, &mut no), Ok(LO + 7 * PAGE));
        assert_eq!(
            vmar.allocate(lowest(PAGE), RW, &mut no),
            Err(VmarError::NoSpace)
        );
    }

    #[test]
    fn guards_are_reserved_and_never_resolve() {
        let mut vmar = vmar::<8>();
        let stack =
            Attrs::new(Kind::Stack, Perms::READ.union(Perms::WRITE)).with_guards(true, true);
        vmar.reserve(LO + 4 * PAGE, 2 * PAGE, stack).unwrap();
        let region = vmar.regions()[0];
        assert_eq!((region.lo(), region.hi()), (LO + 3 * PAGE, LO + 7 * PAGE));
        assert!(matches!(vmar.find(LO + 3 * PAGE), Hit::Guard(_)));
        assert!(matches!(vmar.find(LO + 6 * PAGE + 5), Hit::Guard(_)));
        assert!(matches!(vmar.find(LO + 4 * PAGE), Hit::Region(_)));
        assert_eq!(vmar.find(LO + 2 * PAGE), Hit::Hole);
        assert_eq!(vmar.find(LO + 7 * PAGE), Hit::Hole);
        assert_eq!(
            vmar.check_fault(LO + 3 * PAGE, Access::Write),
            Err(FaultError::Guard)
        );
        assert_eq!(
            vmar.reserve(LO + 3 * PAGE, PAGE, RW),
            Err(VmarError::Overlap)
        );
        assert_eq!(
            vmar.reserve(LO + 6 * PAGE, PAGE, RW),
            Err(VmarError::Overlap)
        );
        vmar.reserve(LO + 2 * PAGE, PAGE, RW).unwrap();
        vmar.reserve(LO + 7 * PAGE, PAGE, RW).unwrap();
    }

    #[test]
    fn guard_below_the_bottom_of_the_range_is_out_of_range() {
        let mut vmar = vmar::<4>();
        let guarded = RW.with_guards(true, false);
        assert_eq!(vmar.reserve(LO, PAGE, guarded), Err(VmarError::OutOfRange));
        vmar.reserve(LO + PAGE, PAGE, guarded).unwrap();
    }

    #[test]
    fn allocation_with_guards_keeps_them_inside_the_hole() {
        let mut vmar = Vmar::new([Region::EMPTY; 4], LO, LO + 4 * PAGE).unwrap();
        let guarded = RW.with_guards(true, true);
        assert_eq!(
            vmar.allocate(lowest(2 * PAGE), guarded, &mut no_entropy()),
            Ok(LO + PAGE)
        );
        assert_eq!(
            vmar.allocate(lowest(PAGE), RW, &mut no_entropy()),
            Err(VmarError::NoSpace)
        );
        let region = vmar.regions()[0];
        assert_eq!((region.lo(), region.hi()), (LO, LO + 4 * PAGE));
    }

    #[test]
    fn fault_decision_follows_the_region_permissions() {
        let mut vmar = vmar::<4>();
        let code = Attrs::new(Kind::Image, Perms::READ.union(Perms::EXECUTE));
        vmar.reserve(LO, PAGE, code).unwrap();
        vmar.reserve(LO + PAGE, PAGE, RW).unwrap();
        assert!(vmar.check_fault(LO, Access::Execute).is_ok());
        assert!(vmar.check_fault(LO + 8, Access::Read).is_ok());
        assert_eq!(
            vmar.check_fault(LO, Access::Write),
            Err(FaultError::Permission)
        );
        assert_eq!(
            vmar.check_fault(LO + PAGE, Access::Execute),
            Err(FaultError::Permission)
        );
        assert_eq!(
            vmar.check_fault(LO + 2 * PAGE, Access::Read),
            Err(FaultError::NoRegion)
        );
        assert_eq!(vmar.check_fault(0, Access::Read), Err(FaultError::NoRegion));
        assert_eq!(
            vmar.check_fault(u64::MAX, Access::Read),
            Err(FaultError::NoRegion)
        );
    }

    fn lazy_vmar() -> Vmar<[Region; 4]> {
        let mut vmar = vmar::<4>();
        let stack = RW.with_guards(true, true).lazy();
        vmar.reserve(LO + PAGE, 4 * PAGE, stack).unwrap();
        let rodata = Attrs::new(Kind::Image, Perms::READ).lazy();
        vmar.reserve(LO + 8 * PAGE, PAGE, rodata).unwrap();
        vmar.reserve(LO + 10 * PAGE, PAGE, RW).unwrap();
        vmar
    }

    #[test]
    fn a_lazy_region_resolves_an_absent_page_by_the_access() {
        let vmar = lazy_vmar();
        let at = LO + 2 * PAGE + 24;
        assert_eq!(
            vmar.plan_fault(at, Access::Read, Mapped::Absent),
            Ok(FaultPlan::MapZero)
        );
        assert_eq!(
            vmar.plan_fault(at, Access::Write, Mapped::Absent),
            Ok(FaultPlan::MapFresh)
        );
    }

    #[test]
    fn a_write_over_the_zero_page_copies_it_and_a_read_just_retries() {
        let vmar = lazy_vmar();
        let at = LO + 3 * PAGE;
        assert_eq!(
            vmar.plan_fault(at, Access::Write, Mapped::ZeroPage),
            Ok(FaultPlan::CopyZero)
        );
        assert_eq!(
            vmar.plan_fault(at, Access::Read, Mapped::ZeroPage),
            Ok(FaultPlan::Retry)
        );
    }

    #[test]
    fn a_page_another_cpu_already_resolved_only_retries() {
        let vmar = lazy_vmar();
        for access in [Access::Read, Access::Write] {
            assert_eq!(
                vmar.plan_fault(LO + PAGE, access, Mapped::Private),
                Ok(FaultPlan::Retry)
            );
        }
    }

    #[test]
    fn the_plan_keeps_the_region_rules_guards_holes_and_permissions() {
        let vmar = lazy_vmar();
        assert_eq!(
            vmar.plan_fault(LO, Access::Write, Mapped::Absent),
            Err(FaultError::Guard)
        );
        assert_eq!(
            vmar.plan_fault(LO + 5 * PAGE, Access::Read, Mapped::Absent),
            Err(FaultError::Guard)
        );
        assert_eq!(
            vmar.plan_fault(LO + 6 * PAGE, Access::Read, Mapped::Absent),
            Err(FaultError::NoRegion)
        );
        assert_eq!(
            vmar.plan_fault(LO + 8 * PAGE, Access::Write, Mapped::Absent),
            Err(FaultError::Permission)
        );
        assert_eq!(
            vmar.plan_fault(LO + 2 * PAGE, Access::Execute, Mapped::Absent),
            Err(FaultError::Permission)
        );
        assert_eq!(
            vmar.plan_fault(LO + 8 * PAGE, Access::Read, Mapped::Absent),
            Ok(FaultPlan::MapZero)
        );
    }

    #[test]
    fn a_region_that_is_not_lazy_never_creates_a_page() {
        let vmar = lazy_vmar();
        let at = LO + 10 * PAGE;
        for access in [Access::Read, Access::Write] {
            assert_eq!(
                vmar.plan_fault(at, access, Mapped::Absent),
                Err(FaultError::Unbacked)
            );
        }
        assert_eq!(
            vmar.plan_fault(at, Access::Read, Mapped::Private),
            Ok(FaultPlan::Retry)
        );
    }

    #[test]
    fn object_page_counts_from_the_backing_offset() {
        let backing = Backing {
            object: NonZeroU32::new(7).unwrap(),
            page_offset: 10,
        };
        let mut vmar = vmar::<4>();
        vmar.reserve(LO, 4 * PAGE, RW.with_backing(backing))
            .unwrap();
        let region = vmar.check_fault(LO + 2 * PAGE + 9, Access::Read).unwrap();
        assert_eq!(
            region.object_page(LO + 2 * PAGE + 9),
            Some((backing.object, 12))
        );
        assert_eq!(region.object_page(LO + 4 * PAGE), None);
        vmar.unmap(LO, PAGE).unwrap();
        let region = vmar.regions()[0];
        assert_eq!(region.object_page(LO + PAGE), Some((backing.object, 11)));
    }

    #[test]
    fn backing_that_runs_past_the_u32_page_offset_is_refused() {
        let backing = Backing {
            object: NonZeroU32::new(1).unwrap(),
            page_offset: u32::MAX - 1,
        };
        let mut vmar = vmar::<4>();
        let attrs = RW.with_backing(backing);
        assert_eq!(
            vmar.reserve(LO, 3 * PAGE, attrs),
            Err(VmarError::OutOfRange)
        );
        vmar.reserve(LO, PAGE, attrs).unwrap();
        let got = vmar.allocate(lowest(2 * PAGE), attrs, &mut no_entropy());
        assert_eq!(got, Err(VmarError::OutOfRange));
        assert!(
            vmar.allocate(lowest(PAGE), attrs, &mut no_entropy())
                .is_ok()
        );
    }

    #[test]
    fn unmap_removes_trims_and_splits() {
        let mut vmar = vmar::<8>();
        let stack =
            Attrs::new(Kind::Stack, Perms::READ.union(Perms::WRITE)).with_guards(true, true);
        vmar.reserve(LO + 10 * PAGE, 8 * PAGE, stack).unwrap();
        vmar.unmap(LO + 13 * PAGE, 2 * PAGE).unwrap();
        let [low, high] = [vmar.regions()[0], vmar.regions()[1]];
        assert_eq!((low.start, low.end), (LO + 10 * PAGE, LO + 13 * PAGE));
        assert_eq!((high.start, high.end), (LO + 15 * PAGE, LO + 18 * PAGE));
        assert!(low.attrs.guard_below && !low.attrs.guard_above);
        assert!(!high.attrs.guard_below && high.attrs.guard_above);
        vmar.unmap(LO + 12 * PAGE, 2 * PAGE).unwrap();
        assert_eq!(vmar.regions()[0].end, LO + 12 * PAGE);
        vmar.unmap(LO + 15 * PAGE, PAGE).unwrap();
        assert_eq!(vmar.regions()[1].start, LO + 16 * PAGE);
        vmar.unmap(LO, 100 * PAGE).unwrap();
        assert!(vmar.is_empty());
    }

    #[test]
    fn unmap_over_holes_and_guards_changes_nothing() {
        let mut vmar = vmar::<4>();
        let guarded = RW.with_guards(true, true);
        vmar.reserve(LO + 4 * PAGE, PAGE, guarded).unwrap();
        let before = vmar.regions().to_vec();
        vmar.unmap(LO, 4 * PAGE).unwrap();
        vmar.unmap(LO + 5 * PAGE, 2 * PAGE).unwrap();
        assert_eq!(vmar.regions(), before);
    }

    #[test]
    fn unmap_refuses_bad_ranges() {
        let mut vmar = vmar::<4>();
        assert_eq!(vmar.unmap(LO, 0), Err(VmarError::Empty));
        assert_eq!(vmar.unmap(LO + 1, PAGE), Err(VmarError::Misaligned));
        assert_eq!(vmar.unmap(LO, PAGE + 1), Err(VmarError::Misaligned));
        assert_eq!(vmar.unmap(LO - PAGE, PAGE), Err(VmarError::OutOfRange));
        assert_eq!(vmar.unmap(HI - PAGE, 2 * PAGE), Err(VmarError::OutOfRange));
        let wrap = !(PAGE - 1);
        assert_eq!(vmar.unmap(HI - PAGE, wrap), Err(VmarError::OutOfRange));
    }

    #[test]
    fn splitting_needs_a_free_slot_and_leaves_the_tree_alone_without_one() {
        let mut vmar = vmar::<1>();
        vmar.reserve(LO, 3 * PAGE, RW).unwrap();
        let before = vmar.regions().to_vec();
        assert_eq!(vmar.unmap(LO + PAGE, PAGE), Err(VmarError::Full));
        assert_eq!(vmar.regions(), before);
        vmar.unmap(LO + 2 * PAGE, PAGE).unwrap();
        assert_eq!(vmar.regions()[0].end, LO + 2 * PAGE);
    }

    #[test]
    fn clear_empties_the_tree() {
        let mut vmar = vmar::<4>();
        vmar.reserve(LO, PAGE, RW).unwrap();
        vmar.clear();
        assert!(vmar.is_empty());
        assert_eq!(vmar.find(LO), Hit::Hole);
        vmar.reserve(LO, PAGE, RW).unwrap();
    }

    #[test]
    fn boxed_slice_storage_works_too() {
        let storage = vec![Region::EMPTY; 3].into_boxed_slice();
        let mut vmar = Vmar::new(storage, LO, HI).unwrap();
        assert_eq!(vmar.capacity(), 3);
        vmar.reserve(LO, PAGE, RW).unwrap();
        assert!(matches!(vmar.find(LO), Hit::Region(_)));
    }

    #[test]
    fn random_placement_is_uniform_over_the_valid_positions() {
        let mut vmar = Vmar::new([Region::EMPTY; 8], LO, LO + 12 * PAGE).unwrap();
        vmar.reserve(LO + 4 * PAGE, 2 * PAGE, RW).unwrap();
        let fit = Fit {
            placement: Placement::Random,
            ..lowest(2 * PAGE)
        };
        let valid = [0, 1, 2, 6, 7, 8, 9, 10].map(|page| LO + page * PAGE);
        let mut seen = [0_u32; 8];
        let mut source = XorShift::new(0x5EED);
        let mut rng = || source.next();
        for _ in 0..16_000 {
            let mut copy = Vmar::new([Region::EMPTY; 8], LO, LO + 12 * PAGE).unwrap();
            copy.reserve(LO + 4 * PAGE, 2 * PAGE, RW).unwrap();
            let got = copy.allocate(fit, RW, &mut rng).unwrap();
            let slot = valid
                .iter()
                .position(|&v| v == got)
                .expect("posição inválida");
            seen[slot] += 1;
        }
        for (slot, count) in seen.iter().enumerate() {
            assert!((1_700..=2_300).contains(count), "posição {slot}: {count}");
        }
        assert_eq!(vmar.len(), 1);
    }

    #[test]
    fn random_placement_picks_by_the_scripted_draw() {
        let mut vmar = Vmar::new([Region::EMPTY; 8], LO, LO + 10 * PAGE).unwrap();
        vmar.reserve(LO + 3 * PAGE, PAGE, RW).unwrap();
        let fit = Fit {
            placement: Placement::Random,
            ..lowest(PAGE)
        };
        // 9 posições; o sorteio k = alto(palavra * 9) com a palavra no limite de cada fatia.
        let slice = u64::MAX / 9 + 1;
        let mut word = 4 * slice;
        let got = vmar.allocate(fit, RW, &mut || word).unwrap();
        assert_eq!(got, LO + 5 * PAGE);
        word = 0;
        assert_eq!(vmar.allocate(fit, RW, &mut || word), Ok(LO));
        word = u64::MAX;
        assert_eq!(vmar.allocate(fit, RW, &mut || word), Ok(LO + 9 * PAGE));
    }

    #[test]
    fn rejection_is_bounded_even_for_a_stuck_source() {
        let mut calls = 0;
        let mut stuck = || {
            calls += 1;
            0
        };
        let bound = (1_u64 << 63) + 1;
        let got = uniform_below(bound, &mut stuck);
        assert!(got < bound);
        assert!(calls <= REJECTION_TRIES + 1, "{calls}");
    }

    #[test]
    fn uniform_below_covers_the_whole_bound_without_bias() {
        let mut source = XorShift::new(99);
        let mut rng = || source.next();
        let mut seen = [0_u32; 7];
        for _ in 0..70_000 {
            seen[usize::try_from(uniform_below(7, &mut rng)).unwrap()] += 1;
        }
        assert!(
            seen.iter().all(|&c| (9_400..=10_600).contains(&c)),
            "{seen:?}"
        );
        let mut one = || panic!("bound 1 não consome palavra");
        assert_eq!(uniform_below(1, &mut one), 0);
    }

    #[test]
    fn aslr_base_has_28_bits_of_page_entropy() {
        let floor = 0x10_0000_0000;
        let limit = 0x7000_0000_0000;
        let mut source = XorShift::new(1234);
        let mut rng = || source.next();
        let mut high = 0;
        let mut seen_bits = 0_u64;
        for _ in 0..20_000 {
            let base = aslr_base(floor, limit, &mut rng).unwrap();
            assert!(base % PAGE == 0 && (floor..floor + (1 << 40)).contains(&base));
            seen_bits |= (base - floor) / PAGE;
            high += u32::from(base - floor >= 1 << 39);
        }
        assert_eq!(seen_bits >> ASLR_BITS, 0);
        assert_eq!(seen_bits.count_ones(), ASLR_BITS);
        assert!((9_500..=10_500).contains(&high), "{high}");
    }

    #[test]
    fn aslr_base_refuses_an_interval_without_a_whole_page() {
        let mut rng = || panic!("sem posição, sem sorteio");
        assert_eq!(aslr_base(LO, LO + 1, &mut rng), None);
        assert_eq!(aslr_base(LO, LO + PAGE - 1, &mut rng), None);
        assert_eq!(aslr_bits(LO, LO + 1), 0);
        let mut rng = || u64::MAX;
        assert_eq!(aslr_base(LO, LO + PAGE + 1, &mut rng), Some(LO));
    }

    #[test]
    fn aslr_bits_reports_the_effective_entropy() {
        assert_eq!(aslr_bits(LO, HI), ASLR_BITS);
        assert_eq!(aslr_bits(LO, LO + (1 << ASLR_BITS) * PAGE), ASLR_BITS);
        assert_eq!(aslr_bits(LO, LO + 1024 * PAGE), 10);
        assert_eq!(aslr_bits(LO, LO + 1023 * PAGE), 9);
        assert_eq!(aslr_bits(LO, LO + PAGE), 0);
        assert_eq!(aslr_bits(LO, LO), 0);
        assert_eq!(aslr_bits(LO + 1, HI), 0);
    }

    #[test]
    fn uniform_below_zero_does_not_divide_by_zero() {
        let mut never = || panic!("bound 0 não consome palavra");
        assert_eq!(uniform_below(0, &mut never), 0);
    }

    #[test]
    fn aslr_base_clamps_to_the_limit_and_refuses_bad_input() {
        let mut rng = || u64::MAX;
        assert_eq!(aslr_base(LO, LO + PAGE, &mut rng), Some(LO));
        assert_eq!(aslr_base(LO, LO, &mut rng), None);
        assert_eq!(aslr_base(LO + 1, HI, &mut rng), None);
        let last = aslr_base(0, !(PAGE - 1), &mut rng).unwrap();
        assert_eq!(last, ((1 << ASLR_BITS) - 1) * PAGE);
    }

    /// Modelo ingênuo: uma lista de regiões ordenada por início, sem nada esperto.
    struct Model {
        lo: u64,
        hi: u64,
        cap: usize,
        regions: Vec<Region>,
    }

    impl Model {
        fn reserve(&mut self, start: u64, len: u64, attrs: Attrs) -> Result<(), VmarError> {
            let (nlo, nhi) = (start - attrs.below(), start + len + attrs.above());
            if nlo < self.lo || nhi > self.hi {
                return Err(VmarError::OutOfRange);
            }
            if self.regions.iter().any(|r| r.lo() < nhi && nlo < r.hi()) {
                return Err(VmarError::Overlap);
            }
            if self.regions.len() == self.cap {
                return Err(VmarError::Full);
            }
            self.regions.push(Region {
                start,
                end: start + len,
                attrs,
            });
            self.regions.sort_by_key(|r| r.start);
            Ok(())
        }

        fn candidates(&self, fit: &Fit, attrs: Attrs) -> Vec<u64> {
            let mut out = Vec::new();
            let mut page = self.lo;
            while page < self.hi {
                let p = page;
                page += PAGE;
                if !p.is_multiple_of(fit.align) || p < fit.window.0 + attrs.below() {
                    continue;
                }
                let (nlo, nhi) = (p - attrs.below(), p + fit.len + attrs.above());
                if nlo < self.lo || nhi > self.hi.min(fit.window.1) {
                    continue;
                }
                if self.regions.iter().any(|r| r.lo() < nhi && nlo < r.hi()) {
                    continue;
                }
                out.push(p);
            }
            out
        }

        fn unmap(&mut self, start: u64, len: u64) -> Result<(), VmarError> {
            let end = start + len;
            let splits = self.regions.iter().any(|r| r.start < start && end < r.end);
            if splits && self.regions.len() == self.cap {
                return Err(VmarError::Full);
            }
            let old = core::mem::take(&mut self.regions);
            for region in old {
                if region.end <= start || end <= region.start {
                    self.regions.push(region);
                } else {
                    if region.start < start {
                        let mut low = region;
                        low.end = start;
                        low.attrs.guard_above = false;
                        self.regions.push(low);
                    }
                    if end < region.end {
                        self.regions.push(cut_front(region, end));
                    }
                }
            }
            Ok(())
        }

        fn find(&self, addr: u64) -> Option<(bool, Region)> {
            self.regions
                .iter()
                .find(|r| r.lo() <= addr && addr < r.hi())
                .map(|&r| (r.start <= addr && addr < r.end, r))
        }
    }

    fn random_attrs(rng: &mut XorShift, serial: u32) -> Attrs {
        let perms = [
            Perms::NONE,
            Perms::READ,
            Perms::READ.union(Perms::WRITE),
            Perms::READ.union(Perms::EXECUTE),
        ][rng.below(4)];
        let kind = [Kind::Image, Kind::Stack, Kind::Mapping][rng.below(3)];
        let attrs = Attrs::new(kind, perms).with_guards(rng.below(4) == 0, rng.below(4) == 0);
        match NonZeroU32::new(serial) {
            Some(object) if rng.below(3) == 0 => attrs.with_backing(Backing {
                object,
                page_offset: u32::try_from(rng.below(1000)).unwrap(),
            }),
            _ => attrs,
        }
    }

    fn assert_same(vmar: &Vmar<Vec<Region>>, model: &Model, ctx: &str) {
        assert_eq!(vmar.regions(), model.regions, "{ctx}");
        let regions = vmar.regions();
        for pair in regions.windows(2) {
            assert!(pair[0].hi() <= pair[1].lo(), "{ctx}: sobreposição");
        }
        for region in regions {
            assert!(region.start < region.end && region.lo() >= vmar.lo && region.hi() <= vmar.hi);
            assert!(!region.attrs.perms.is_write_execute(), "{ctx}");
        }
    }

    fn run_against_model(seed: u64, pages: u64, cap: usize, steps: u32) {
        let (lo, hi) = (LO, LO + pages * PAGE);
        let mut vmar = Vmar::new(vec![Region::EMPTY; cap], lo, hi).unwrap();
        let mut model = Model {
            lo,
            hi,
            cap,
            regions: Vec::new(),
        };
        let mut rng = XorShift::new(seed);
        let mut entropy = XorShift::new(seed ^ 0xABCD);
        for step in 0..steps {
            let ctx = format!("semente {seed:#x}, passo {step}");
            let page = u64::try_from(rng.below(usize::try_from(pages).unwrap() + 2)).unwrap();
            let len = (1 + u64::try_from(rng.below(6)).unwrap()) * PAGE;
            let attrs = random_attrs(&mut rng, step + 1);
            match rng.below(10) {
                0..=2 => {
                    let start = lo + page * PAGE - PAGE;
                    let got = vmar.reserve(start, len, attrs);
                    assert_eq!(got, model.reserve(start, len, attrs), "{ctx}");
                }
                3..=5 => {
                    let align = PAGE << rng.below(5);
                    let placement = if rng.below(2) == 0 {
                        Placement::Lowest
                    } else {
                        Placement::Random
                    };
                    let window = if rng.below(3) == 0 {
                        (lo + page * PAGE, hi)
                    } else {
                        (0, u64::MAX)
                    };
                    let fit = Fit {
                        len,
                        align,
                        window,
                        placement,
                    };
                    let candidates = model.candidates(&fit, attrs);
                    let mut draws = 0;
                    let mut rng_fn = || {
                        draws += 1;
                        entropy.next()
                    };
                    let got = vmar.allocate(fit, attrs, &mut rng_fn);
                    if model.regions.len() == cap {
                        assert_eq!(got, Err(VmarError::Full), "{ctx}");
                    } else if candidates.is_empty() {
                        assert_eq!(got, Err(VmarError::NoSpace), "{ctx}");
                    } else {
                        let start = got.unwrap_or_else(|e| panic!("{ctx}: {e:?}"));
                        assert!(candidates.contains(&start), "{ctx}");
                        if placement == Placement::Lowest {
                            assert_eq!(start, candidates[0], "{ctx}");
                            assert_eq!(draws, 0, "{ctx}");
                        }
                        model.reserve(start, len, attrs).unwrap();
                    }
                }
                6..=7 => {
                    let start = lo + page * PAGE - PAGE;
                    let got = vmar.unmap(start, len);
                    if start < lo || start + len > hi {
                        assert_eq!(got, Err(VmarError::OutOfRange), "{ctx}");
                    } else {
                        assert_eq!(got, model.unmap(start, len), "{ctx}");
                    }
                }
                _ => {
                    let addr = lo - PAGE
                        + u64::try_from(rng.below(usize::try_from(pages + 2).unwrap() * 4096))
                            .unwrap();
                    let found = vmar.find(addr);
                    match (found, model.find(addr)) {
                        (Hit::Hole, None) => {}
                        (Hit::Region(r), Some((true, m))) | (Hit::Guard(r), Some((false, m))) => {
                            assert_eq!(*r, m, "{ctx}");
                        }
                        (got, want) => panic!("{ctx}: {got:?} contra {want:?}"),
                    }
                }
            }
            assert_same(&vmar, &model, &ctx);
        }
    }

    #[test]
    fn thousands_of_random_operations_match_the_naive_model() {
        for (seed, pages, cap) in [
            (1, 64, 6),
            (2, 200, 16),
            (3, 40, 3),
            (0xDEAD, 512, 64),
            (7, 24, 24),
        ] {
            run_against_model(seed, pages, cap, 6_000);
        }
    }
}
