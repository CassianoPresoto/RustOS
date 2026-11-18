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
    /// Acessível ao anel 3 ou a EL0; só na metade de usuário (SDD 17).
    pub user: bool,
    pub cache: CacheMode,
}

impl PageFlags {
    /// `.text`: leitura e execução.
    pub const CODE: Self = Self {
        writable: false,
        executable: true,
        user: false,
        cache: CacheMode::WriteBack,
    };
    /// `.rodata`: só leitura.
    pub const RODATA: Self = Self {
        writable: false,
        executable: false,
        user: false,
        cache: CacheMode::WriteBack,
    };
    /// `.data`, `.bss`, pilhas, mapa direto: leitura e escrita, sem execução.
    pub const DATA: Self = Self {
        writable: true,
        executable: false,
        user: false,
        cache: CacheMode::WriteBack,
    };
    pub const DEVICE: Self = Self {
        writable: true,
        executable: false,
        user: false,
        cache: CacheMode::Device,
    };
    pub const FRAMEBUFFER: Self = Self {
        writable: true,
        executable: false,
        user: false,
        cache: CacheMode::WriteCombining,
    };

    /// Código de usuário: leitura e execução.
    pub const USER_CODE: Self = Self {
        user: true,
        ..Self::CODE
    };
    /// Dado somente leitura de usuário.
    pub const USER_RODATA: Self = Self {
        user: true,
        ..Self::RODATA
    };
    /// Dado, pilha e heap de usuário: leitura e escrita, sem execução.
    pub const USER_DATA: Self = Self {
        user: true,
        ..Self::DATA
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
    /// Gravável e executável ao mesmo tempo: o mapeador recusa (guidelines 09).
    WriteExecute(VirtAddr),
    /// Folha de usuário fora de `[USER_MIN, USER_TOP)`, ou do kernel na metade de usuário.
    UserRange(VirtAddr),
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

/// Memória das tabelas de página: o que o mapeador lê e escreve, entrada a entrada.
/// O kernel a liga ao mapa direto; os testes, a um `HashMap`.
pub trait TableMemory {
    fn read(&self, table: PhysAddr, index: usize) -> u64;
    fn write(&mut self, table: PhysAddr, index: usize, entry: u64);
    /// Zera a tabela inteira (`PAGE_SIZE` bytes).
    fn zero(&mut self, table: PhysAddr);
}

/// Formato dos descritores de uma ISA: só a codificação. A caminhada de 4 níveis é
/// uma só, em `core/mm`, e consulta o formato por esta porta.
pub trait PageFormat {
    /// Entrada de nível intermediário: permissão ampla, quem restringe é a folha.
    fn table_entry(next: PhysAddr) -> u64;
    /// Como `table_entry`, para a hierarquia de uma folha de usuário (no x86_64 o `U/S` vale
    /// em todos os níveis).
    fn user_table_entry(next: PhysAddr) -> u64 {
        Self::table_entry(next)
    }
    /// Folha de `PAGE_SIZE` (último nível).
    fn page_entry(phys: PhysAddr, flags: PageFlags) -> u64;
    /// Folha de `LARGE_PAGE_SIZE` (um nível acima do último).
    fn large_entry(phys: PhysAddr, flags: PageFlags) -> u64;
    /// A entrada mapeia alguma coisa (presente, válida).
    fn is_valid(entry: u64) -> bool;
    /// A entrada válida do nível `level` (0 a 2) aponta para tabela, não para folha.
    fn is_table(entry: u64, level: usize) -> bool;
    fn table_address(entry: u64) -> PhysAddr;
    /// Endereço e permissão de uma folha. `large` diz o tamanho dela.
    fn decode_leaf(entry: u64, large: bool) -> (PhysAddr, PageFlags);
    /// Permissão efetiva ao descer por uma entrada: escrita é o E dos níveis, e
    /// não-execução, o OU. Recebe e devolve `(gravável, sem execução)`.
    fn restrict(writable: bool, no_execute: bool, entry: u64) -> (bool, bool);
    /// Endereço virtual que a caminhada resolveu, estendido ao canônico da metade alta.
    fn canonical(virt: u64) -> u64;
}

/// O que o mapeador pede à CPU: publicar uma entrada nova e trocar de espaço.
pub trait MmuControl {
    /// A entrada de `virt` mudou: a CPU tem de enxergá-la (`invlpg`; barreiras).
    fn entry_changed(&self, virt: VirtAddr);
    /// Passa a traduzir pela raiz `root` e descarta o TLB.
    fn activate(&self, root: PhysAddr);
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
            user: false,
            cache: CacheMode::WriteBack,
        };
        assert!(both.violates_wx());
    }

    const _: () = {
        assert!(PageFlags::USER_CODE.user && PageFlags::USER_CODE.executable);
        assert!(!PageFlags::USER_CODE.writable);
        assert!(PageFlags::USER_DATA.writable && !PageFlags::USER_DATA.executable);
        assert!(!PageFlags::USER_RODATA.writable && !PageFlags::USER_RODATA.executable);
        assert!(!PageFlags::DATA.user && !PageFlags::DEVICE.user);
    };

    #[test]
    fn user_flags_never_write_and_execute() {
        for flags in [
            PageFlags::USER_CODE,
            PageFlags::USER_RODATA,
            PageFlags::USER_DATA,
        ] {
            assert!(!flags.violates_wx());
        }
    }

    #[test]
    fn large_page_is_two_mebibytes() {
        assert_eq!(LARGE_PAGE_SIZE, 2 * 1024 * 1024);
    }
}
