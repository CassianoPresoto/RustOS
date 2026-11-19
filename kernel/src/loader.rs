//! O carregador de executável do initramfs (SDD 17, fatia 4b): acha o arquivo no `cpio`, valida o
//! ELF com o `core/elf`, copia cada `PT_LOAD` para frames novos, registra as regiões no `Vmar` do
//! processo e monta a pilha inicial com guarda. A pilha e o `.bss` de páginas inteiras são regiões
//! preguiçosas. Só o programa que o kernel lança nasce aqui; o filho do `init` vem da ABI.

use alloc::boxed::Box;
use alloc::vec;

use rustos_cpio as cpio;
use rustos_elf::{self as elf, Image, Limits, Segment};
use rustos_hal::abi::{
    AT_RANDOM_LEN, EXEC_MAX_LOADS_BYTES, InitialStack, StartInfo, USER_MIN, USER_TOP,
};
use rustos_hal::{MapError, PAGE_SIZE, PageFlags};
use rustos_mm::vmar::{Attrs, Kind, Perms, Region, Vmar, VmarError, aslr_base};

use crate::random;
use crate::uspace::UserSpace;

/// A árvore de regiões de um processo: vetor ordenado de capacidade fixa (DEBT-0075).
pub type RegionTree = Vmar<Box<[Region]>>;

/// Regiões que cabem na árvore: no máximo 16 `PT_LOAD` e a pilha, com folga para os mapeamentos da
/// 4c; 32 regiões custam 1 KiB por processo. O `.bss` preguiçoso não ganha região à parte.
pub const REGION_CAPACITY: usize = 32;

/// A pilha inicial, com uma página de guarda abaixo e outra acima.
pub const STACK_PAGES: u64 = rustos_hal::abi::USER_STACK_PAGES;
/// Onde o início da pilha cai: `2^28` páginas (1 TiB) a partir de `0x6000_0000_0000`.
const STACK_WINDOW: (u64, u64) = (0x6000_0000_0000, 0x6100_0000_0000);

pub const MACHINE: u16 = if cfg!(target_arch = "x86_64") {
    elf::EM_X86_64
} else {
    elf::EM_AARCH64
};

/// Por que um programa não virou processo.
#[derive(Debug)]
pub enum LoadError {
    /// O initramfs não tem o arquivo.
    Missing,
    Archive(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] cpio::Error),
    Elf(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] elf::Error),
    Map(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] MapError),
    Region(#[allow(dead_code, reason = "lido pelo Debug da mensagem")] VmarError),
    /// O gerador do kernel ainda não foi semeado: sem `AT_RANDOM` nem base da pilha.
    NoEntropy,
    /// O topo da pilha não coube no bloco inicial: não acontece com a janela fixa.
    StackLayout,
}

impl From<MapError> for LoadError {
    fn from(error: MapError) -> Self {
        Self::Map(error)
    }
}

impl From<VmarError> for LoadError {
    fn from(error: VmarError) -> Self {
        Self::Region(error)
    }
}

/// O que a imagem carregada diz ao resto do `Process`.
#[derive(Clone, Copy, Debug)]
pub struct Loaded {
    pub entry: u64,
    pub segments: usize,
    pub phdr: Option<u64>,
    pub phnum: u16,
}

/// A árvore vazia de um processo novo, no heap (o armazenamento não passa pela pilha do kernel).
pub fn new_tree() -> Result<RegionTree, VmarError> {
    let storage = vec![Region::EMPTY; REGION_CAPACITY].into_boxed_slice();
    Vmar::new(storage, USER_MIN, USER_TOP)
}

/// O arquivo `name` do initramfs; com nome repetido vale o último (`core/cpio`).
pub fn find<'a>(archive: &'a [u8], name: &[u8]) -> Result<&'a [u8], LoadError> {
    match cpio::find(archive, name) {
        Ok(Some(entry)) => Ok(entry.data),
        Ok(None) => Err(LoadError::Missing),
        Err(error) => Err(LoadError::Archive(error)),
    }
}

/// Os limites da validação: a ISA do kernel e a faixa do usuário, de `hal::abi`.
pub const fn limits() -> Limits {
    Limits {
        machine: MACHINE,
        user_min: USER_MIN,
        user_top: USER_TOP,
        page_size: PAGE_SIZE,
        max_loads_bytes: EXEC_MAX_LOADS_BYTES,
    }
}

/// Valida `file` e mapeia os `PT_LOAD` em `space`, registrando cada faixa em `tree`.
pub fn load_image(
    space: &mut UserSpace,
    tree: &mut RegionTree,
    file: &[u8],
) -> Result<Loaded, LoadError> {
    let image = elf::parse(file, &limits()).map_err(LoadError::Elf)?;
    for segment in image.loads() {
        map_segment(space, tree, file, segment)?;
    }
    Ok(summary(&image))
}

fn summary(image: &Image) -> Loaded {
    Loaded {
        entry: image.entry,
        segments: image.loads().len(),
        phdr: image.phdr,
        phnum: image.phnum,
    }
}

/// Aloca os frames das páginas que têm bytes do arquivo (zerados, com o começo da primeira página e
/// a cauda delas em zero) e mapeia com a permissão do segmento. As páginas só de `.bss` de um
/// segmento gravável ficam sem frame: a região é preguiçosa.
fn map_segment(
    space: &mut UserSpace,
    tree: &mut RegionTree,
    file: &[u8],
    segment: &Segment,
) -> Result<(), LoadError> {
    let (start, end) = segment.page_range(PAGE_SIZE);
    let bytes = segment
        .file_bytes(file)
        .ok_or(LoadError::Elf(elf::Error::SegmentOutOfFile))?;
    let lead = usize::try_from(segment.page_offset(PAGE_SIZE)).unwrap_or(0);
    let (flags, perms) = permissions(segment);
    let total = (end - start) / PAGE_SIZE;
    let eager = if flags.writable {
        ((lead + bytes.len()) as u64).div_ceil(PAGE_SIZE).min(total)
    } else {
        total
    };
    let attrs = Attrs::new(Kind::Image, perms);
    tree.reserve(
        start,
        end - start,
        if eager < total { attrs.lazy() } else { attrs },
    )?;
    space.map_at(start, lead, bytes, eager, flags)?;
    space.reserve_lazy(total - eager)?;
    if eager < total {
        space.count_eager(eager);
    }
    Ok(())
}

/// A permissão do segmento nas tabelas e na árvore. O `core/elf` já recusou W+X e segmento sem R.
fn permissions(segment: &Segment) -> (PageFlags, Perms) {
    if segment.executable() {
        (PageFlags::USER_CODE, Perms::READ.union(Perms::EXECUTE))
    } else if segment.writable() {
        (PageFlags::USER_DATA, Perms::READ.union(Perms::WRITE))
    } else {
        (PageFlags::USER_RODATA, Perms::READ)
    }
}

/// Registra a pilha (guarda abaixo e acima, topo sorteado, preguiçosa) e mapeia só a última página,
/// com `argc`, `argv`, `envp` e o auxv; as outras nascem na falta. Devolve o `sp` de entrada, alinhado a 16.
pub fn map_stack(
    space: &mut UserSpace,
    tree: &mut RegionTree,
    loaded: &Loaded,
) -> Result<u64, LoadError> {
    let mut unseeded = false;
    let mut rng = || {
        random::next_u64().unwrap_or_else(|| {
            unseeded = true;
            0
        })
    };
    let drawn = aslr_base(STACK_WINDOW.0, STACK_WINDOW.1, &mut rng);
    if unseeded {
        return Err(LoadError::NoEntropy);
    }
    let start = drawn.ok_or(LoadError::NoEntropy)?;
    let top = start + STACK_PAGES * PAGE_SIZE;
    let attrs = Attrs::new(Kind::Stack, Perms::READ.union(Perms::WRITE))
        .with_guards(true, true)
        .lazy();
    tree.reserve(start, STACK_PAGES * PAGE_SIZE, attrs)?;
    let mut random_bytes = [0_u8; AT_RANDOM_LEN];
    for chunk in random_bytes.as_chunks_mut::<8>().0 {
        let word = random::next_u64().ok_or(LoadError::NoEntropy)?;
        chunk.copy_from_slice(&word.to_le_bytes());
    }
    let mut info = StartInfo {
        entry: loaded.entry,
        phdr: loaded.phdr,
        phent: elf::PHDR_LEN as u64,
        phnum: u64::from(loaded.phnum),
        random: random_bytes,
    };
    rustos_crypto::wipe(&mut random_bytes);
    let built = InitialStack::build(top, &info);
    rustos_crypto::wipe(&mut info.random);
    let mut initial = built.ok_or(LoadError::StackLayout)?;
    let lead = usize::try_from(PAGE_SIZE).unwrap_or(0) - initial.bytes().len();
    let mapped = space.map_at(
        top - PAGE_SIZE,
        lead,
        initial.bytes(),
        1,
        PageFlags::USER_DATA,
    );
    let sp = initial.sp();
    rustos_crypto::wipe(initial.storage_mut());
    mapped?;
    space.reserve_lazy(STACK_PAGES - 1)?;
    space.count_eager(1);
    Ok(sp)
}

/// Quantos bits de página o sorteio da pilha entrega neste espaço: o que o modo de teste relata.
#[cfg(feature = "selftest")]
pub fn stack_aslr_bits() -> u32 {
    rustos_mm::vmar::aslr_bits(STACK_WINDOW.0, STACK_WINDOW.1)
}
