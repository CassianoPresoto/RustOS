//! A memória de um processo: o espaço, a árvore de regiões e os VMOs que ela mapeia (SDD 17, fatia
//! 4c). Um `IrqLock` por processo guarda tudo isto: a falta de página, `vmar_map` e `vmar_unmap`
//! o tomam, e quem o segura espera o *shootdown* das outras CPUs atendendo os pedidos delas.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::num::NonZeroU32;

use rustos_hal::PAGE_SIZE;
use rustos_hal::abi::{MMAP_ASLR_LIMIT, MMAP_FLOOR, MMAP_TOP, Status};
use rustos_mm::vmar::{
    Access, Attrs, Backing, Fit, Kind, Perms, Placement, Region, VmarError, aslr_base,
};

use crate::loader::{REGION_CAPACITY, RegionTree};
use crate::random;
use crate::uspace::{Pages, Resolved, Unresolved, UserSpace};
use crate::vmo::Vmo;

/// Um VMO mapeado: o `Arc` segura os frames enquanto alguma região o usa, e as tabelas de página
/// que o mapeamento pode pedir ficam prometidas até ele sair inteiro.
#[derive(Debug)]
struct Mapping {
    id: NonZeroU32,
    vmo: Arc<Vmo>,
    table_promise: u64,
}

#[derive(Debug)]
pub struct Mem {
    pub space: UserSpace,
    pub regions: RegionTree,
    maps: Vec<Mapping>,
    next_id: u32,
    mmap_base: u64,
}

/// O que `Mem::map` pede: o VMO e a faixa dele, a permissão e onde.
#[derive(Clone, Copy, Debug)]
pub struct MapRequest {
    pub offset_pages: u64,
    pub pages: u64,
    pub perms: Perms,
    pub fixed: Option<u64>,
}

/// A tabela de páginas que `pages` páginas contíguas pedem, no pior caso: uma por 512 e mais uma.
const fn table_frames(pages: u64) -> u64 {
    pages.div_ceil(512) + 1
}

fn vmar_status(error: VmarError) -> Status {
    match error {
        VmarError::Misaligned | VmarError::Empty | VmarError::BadAlignment => Status::InvalidArgs,
        VmarError::WriteExecute | VmarError::Overlap => Status::InvalidArgs,
        VmarError::OutOfRange => Status::OutOfRange,
        VmarError::Full | VmarError::NoSpace => Status::NoMemory,
    }
}

impl Mem {
    /// `mmap_base` é sorteado aqui, uniforme em `2^28` páginas (SDD 17, ASLR); sem entropia no
    /// gerador do kernel a criação falha.
    pub fn new(space: UserSpace, regions: RegionTree) -> Result<Self, Status> {
        let mut seeded = true;
        let mut rng = || {
            random::next_u64().unwrap_or_else(|| {
                seeded = false;
                0
            })
        };
        let base = aslr_base(MMAP_FLOOR, MMAP_ASLR_LIMIT, &mut rng);
        match base.filter(|_| seeded) {
            Some(mmap_base) => Ok(Self {
                space,
                regions,
                maps: Vec::new(),
                next_id: 1,
                mmap_base,
            }),
            None => {
                space.destroy();
                Err(Status::NotSupported)
            }
        }
    }

    /// A falta de página de usuário em `address`.
    pub fn resolve_fault(&mut self, address: u64, access: Access) -> Result<Resolved, Unresolved> {
        let Self {
            space,
            regions,
            maps,
            ..
        } = self;
        space.resolve_fault_with(regions, address, access, &|id| {
            maps.iter()
                .find(|mapping| mapping.id == id)
                .map(|mapping| &mapping.vmo)
        })
    }

    /// Mapeia `request.pages` páginas do VMO. A faixa é fixa (`FIXED`, recusa sobrepor) ou escolhida
    /// a partir do `mmap_base` sorteado. Devolve o endereço. Nada muda se algo falha.
    pub fn map(&mut self, vmo: &Arc<Vmo>, request: MapRequest) -> Result<u64, Status> {
        let len = request.pages * PAGE_SIZE;
        let tables = table_frames(request.pages);
        self.maps.try_reserve(1).map_err(|_| Status::NoMemory)?;
        let id = NonZeroU32::new(self.next_id).ok_or(Status::NoMemory)?;
        let offset = u32::try_from(request.offset_pages).map_err(|_| Status::InvalidArgs)?;
        self.space
            .reserve_promise(tables)
            .map_err(|_| Status::NoMemory)?;
        let attrs = Attrs::new(Kind::Mapping, request.perms).with_backing(Backing {
            object: id,
            page_offset: offset,
        });
        let placed = match request.fixed {
            Some(at) => self.regions.reserve(at, len, attrs).map(|()| at),
            None => {
                let fit = Fit {
                    len,
                    align: PAGE_SIZE,
                    window: (self.mmap_base, MMAP_TOP),
                    placement: Placement::Lowest,
                };
                self.regions
                    .allocate(fit, attrs, &mut || random::next_u64().unwrap_or(0))
            }
        };
        match placed {
            Ok(at) => {
                self.next_id = self.next_id.wrapping_add(1);
                self.maps.push(Mapping {
                    id,
                    vmo: Arc::clone(vmo),
                    table_promise: tables,
                });
                Ok(at)
            }
            Err(error) => {
                self.space.release_promise(tables);
                Err(vmar_status(error))
            }
        }
    }

    /// Solta `[start, start + len)`: as regiões que ele cobre saem inteiras ou aparadas, as páginas
    /// delas com elas (com *shootdown*), e o mapeamento que ficou sem região devolve a promessa e
    /// solta o VMO. `NoMemory` (sem mexer em nada) se partir uma região pede uma vaga que falta.
    pub fn unmap(&mut self, start: u64, len: u64) -> Result<(), Status> {
        let end = start.checked_add(len).ok_or(Status::OutOfRange)?;
        let mut hit = [Region::EMPTY; REGION_CAPACITY];
        let mut count = 0;
        for region in self.regions.regions() {
            if region.end > start && region.start < end {
                hit[count] = *region;
                count += 1;
            }
        }
        self.regions.unmap(start, len).map_err(vmar_status)?;
        for region in &hit[..count] {
            let (from, to) = (region.start.max(start), region.end.min(end));
            let pages = (to - from) / PAGE_SIZE;
            let kind = if region.attrs.backing.is_some() {
                Pages::Borrowed
            } else if region.attrs.lazy {
                Pages::OwnedLazy
            } else {
                Pages::Owned
            };
            self.space.unmap_pages(from, pages, kind);
        }
        self.sweep();
        Ok(())
    }

    /// Solta o mapeamento que nenhuma região usa mais.
    fn sweep(&mut self) {
        let Self {
            maps,
            regions,
            space,
            ..
        } = self;
        maps.retain(|mapping| {
            let used = regions.regions().iter().any(|region| {
                region
                    .attrs
                    .backing
                    .is_some_and(|backing| backing.object == mapping.id)
            });
            if !used {
                space.release_promise(mapping.table_promise);
            }
            used
        });
    }

    /// Há mapeamento ativo de algum VMO (o autoteste).
    #[cfg(feature = "selftest")]
    pub fn mappings(&self) -> usize {
        self.maps.len()
    }

    /// Desfaz o espaço na morte do processo: as páginas de VMO saem das tabelas (os frames são do
    /// VMO), a promessa das tabelas volta e o resto vai com o espaço.
    pub fn destroy(mut self) {
        let mut hit = [Region::EMPTY; REGION_CAPACITY];
        let mut count = 0;
        for region in self.regions.regions() {
            if region.attrs.backing.is_some() {
                hit[count] = *region;
                count += 1;
            }
        }
        for region in &hit[..count] {
            self.space
                .unmap_pages(region.start, region.pages(), Pages::Borrowed);
        }
        for mapping in &self.maps {
            self.space.release_promise(mapping.table_promise);
        }
        self.maps.clear();
        self.space.destroy();
    }

    /// Espera nenhuma CPU rodar o espaço (a morte chama antes de `destroy`).
    pub fn wait_inactive(&self) {
        self.space.wait_inactive();
    }
}
