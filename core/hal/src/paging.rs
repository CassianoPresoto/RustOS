//! Porta de paginação: mapear página virtual em frame físico e ativar o espaço.

use crate::{PhysAddr, VirtAddr};

/// Granularidade de página e de frame em todas as ISAs suportadas hoje.
pub const PAGE_SIZE: u64 = 4096;
/// Página grande: nível 2 do x86_64 (2 MiB) e bloco de nível 2 do aarch64.
pub const LARGE_PAGE_SIZE: u64 = 512 * PAGE_SIZE;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheMode {
    WriteBack,
    /// Escritas juntadas, leitura sem cache: o framebuffer.
    WriteCombining,
    /// Sem cache e sem reordenação: registradores de dispositivo.
    Device,
    /// Combinação que o kernel não escreve; só aparece ao ler tabela alheia.
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageFlags {
    pub writable: bool,
    pub executable: bool,
    pub cache: CacheMode,
}

impl PageFlags {
    /// `.text`: leitura e execução.
    pub const CODE: Self = Self {
        writable: false,
        executable: true,
        cache: CacheMode::WriteBack,
    };
    /// `.rodata`: só leitura.
    pub const RODATA: Self = Self {
        writable: false,
        executable: false,
        cache: CacheMode::WriteBack,
    };
    /// `.data`, `.bss`, pilhas, mapa direto: leitura e escrita, sem execução.
    pub const DATA: Self = Self {
        writable: true,
        executable: false,
        cache: CacheMode::WriteBack,
    };
    pub const DEVICE: Self = Self {
        writable: true,
        executable: false,
        cache: CacheMode::Device,
    };
    pub const FRAMEBUFFER: Self = Self {
        writable: true,
        executable: false,
        cache: CacheMode::WriteCombining,
    };

    /// Gravável e executável ao mesmo tempo: o que o guidelines 09 proíbe.
    pub const fn violates_wx(&self) -> bool {
        self.writable && self.executable
    }
}

const _: () = assert!(PageFlags::CODE.executable && !PageFlags::CODE.writable);
const _: () = assert!(!PageFlags::RODATA.executable && !PageFlags::RODATA.writable);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapError {
    Misaligned,
    AlreadyMapped(VirtAddr),
    /// Uma página grande já cobre o endereço; não se desce mais na tabela.
    HugePageInTheWay(VirtAddr),
    OutOfFrames,
    OutOfAddressSpace,
    /// A CPU não tem o que o mapeamento pede (ex.: NX, write-combining).
    Unsupported(&'static str),
}

/// Fonte de frames físicos para as tabelas de página intermediárias.
pub trait FrameSource {
    fn allocate_frame(&mut self) -> Option<PhysAddr>;
}

pub trait PageMapper {
    /// Mapeia uma página de `PAGE_SIZE`.
    fn map_page<F: FrameSource>(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        flags: PageFlags,
        frames: &mut F,
    ) -> Result<(), MapError>;

    /// Mapeia uma página de `LARGE_PAGE_SIZE`; os dois endereços alinhados a ela.
    fn map_large_page<F: FrameSource>(
        &mut self,
        virt: VirtAddr,
        phys: PhysAddr,
        flags: PageFlags,
        frames: &mut F,
    ) -> Result<(), MapError>;
}

/// Uma folha da tabela de páginas, já decodificada.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mapping {
    pub virt: VirtAddr,
    pub phys: PhysAddr,
    /// `PAGE_SIZE` ou `LARGE_PAGE_SIZE`.
    pub size: u64,
    pub flags: PageFlags,
}

/// Espaço de endereçamento do kernel: monta-se página a página e ativa-se de uma vez.
pub trait AddressSpace: PageMapper {
    /// Passa a traduzir por este espaço e descarta o TLB. Quem o criou garante que
    /// ele mapeia o código, a pilha e os dados em uso (contrato do construtor).
    fn activate(&self);

    /// Chama `visit` em cada folha do espaço, em ordem crescente de endereço.
    fn walk(&self, visit: &mut dyn FnMut(Mapping));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_flags_never_write_and_execute() {
        for flags in [
            PageFlags::CODE,
            PageFlags::RODATA,
            PageFlags::DATA,
            PageFlags::DEVICE,
            PageFlags::FRAMEBUFFER,
        ] {
            assert!(!flags.violates_wx());
        }
        let both = PageFlags {
            writable: true,
            executable: true,
            cache: CacheMode::WriteBack,
        };
        assert!(both.violates_wx());
    }

    #[test]
    fn large_page_is_two_mebibytes() {
        assert_eq!(LARGE_PAGE_SIZE, 2 * 1024 * 1024);
    }
}
