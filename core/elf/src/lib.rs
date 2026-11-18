//! Validação de ELF64 estático: `&[u8]` vira uma `Image` ou um `Error`, sem
//! alocação e sem ler section header (SDD 03, "Formatos de entrada").

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests;

/// Quantos `PT_LOAD` uma imagem pode ter.
pub const MAX_LOADS: usize = 16;
/// Tamanho do cabeçalho ELF64.
pub const EHDR_LEN: usize = 64;
/// Tamanho de um program header ELF64.
pub const PHDR_LEN: usize = 56;
/// Maior tabela de program headers aceita, em bytes.
pub const MAX_PHDR_TABLE: u64 = 65_536;

/// `p_flags`: executável.
pub const PF_X: u32 = 1;
/// `p_flags`: gravável.
pub const PF_W: u32 = 2;
/// `p_flags`: legível.
pub const PF_R: u32 = 4;

/// `e_machine` do x86_64.
pub const EM_X86_64: u16 = 62;
/// `e_machine` do aarch64.
pub const EM_AARCH64: u16 = 183;

const ET_EXEC: u16 = 2;
const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_INTERP: u32 = 3;
const PT_TLS: u32 = 7;

/// O que o chamador sabe e o arquivo não: a ISA e a faixa do usuário.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// `e_machine` esperado.
    pub machine: u16,
    /// Primeiro endereço que o usuário pode mapear.
    pub user_min: u64,
    /// Primeiro endereço acima da faixa do usuário.
    pub user_top: u64,
    /// Tamanho da página; potência de dois.
    pub page_size: u64,
    /// Teto da soma dos `p_memsz` dos `PT_LOAD`.
    pub max_loads_bytes: u64,
}

/// Por que o arquivo foi recusado. Cada variante é uma regra do SDD 03.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// O chamador passou `Limits` inválido (página que não é potência de dois).
    BadLimits,
    /// O arquivo não cobre o cabeçalho ELF.
    Truncated,
    /// `e_ident` não começa com `\x7fELF`.
    BadMagic,
    /// `EI_CLASS` não é `ELFCLASS64`.
    BadClass,
    /// `EI_DATA` não é little-endian.
    BadData,
    /// `EI_VERSION` ou `e_version` não é `EV_CURRENT`.
    BadVersion,
    /// `EI_OSABI` não é `ELFOSABI_NONE`.
    BadOsAbi,
    /// `e_type` não é `ET_EXEC`.
    BadType,
    /// `e_machine` diferente do esperado.
    BadMachine,
    /// `e_phentsize` diferente de 56.
    BadPhentsize,
    /// A tabela de program headers tem 0 bytes ou mais de [`MAX_PHDR_TABLE`].
    BadPhdrTableSize,
    /// `e_phoff + e_phnum * e_phentsize` estoura ou passa do arquivo.
    PhdrTableOutOfFile,
    /// `PT_INTERP`.
    Interp,
    /// `PT_TLS`.
    Tls,
    /// `PT_DYNAMIC`.
    Dynamic,
    /// `PT_LOAD` com `p_memsz` zero.
    EmptyLoad,
    /// `p_filesz > p_memsz`.
    FileszGreaterThanMemsz,
    /// `p_align` que não é 0, 1 nem potência de dois.
    BadAlign,
    /// `p_offset + p_filesz` estoura ou passa do arquivo.
    SegmentOutOfFile,
    /// `p_vaddr + p_memsz` estoura, ou o arredondamento à página estoura.
    VaddrOverflow,
    /// A faixa do segmento, arredondada à página, sai de `[user_min, user_top]`.
    OutOfUserRange,
    /// `p_vaddr` e `p_offset` diferem módulo a página.
    Misaligned,
    /// `PT_LOAD` com W e X.
    WritableAndExecutable,
    /// `PT_LOAD` sem R.
    NotReadable,
    /// Mais de [`MAX_LOADS`] `PT_LOAD`.
    TooManyLoads,
    /// `PT_LOAD` com `p_vaddr` menor que o do anterior.
    Unsorted,
    /// `PT_LOAD` que invade, em páginas, o anterior.
    Overlap,
    /// A soma dos `p_memsz` passa de `max_loads_bytes` (ou estoura).
    TooLarge,
    /// `e_entry` fora do conteúdo de arquivo de todo `PT_LOAD` executável.
    EntryNotExecutable,
}

/// Um `PT_LOAD` já validado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Endereço virtual do primeiro byte do segmento (não é da página).
    pub vaddr: u64,
    /// Bytes do segmento na memória, `.bss` incluído.
    pub memsz: u64,
    /// Posição do primeiro byte do segmento no arquivo.
    pub offset: u64,
    /// Bytes que vêm do arquivo; `filesz <= memsz`.
    pub filesz: u64,
    /// Só `PF_R`, `PF_W` e `PF_X`, nunca W com X.
    pub flags: u32,
}

impl Segment {
    /// Segmento vazio, para preencher o array da `Image`.
    pub const EMPTY: Self = Self {
        vaddr: 0,
        memsz: 0,
        offset: 0,
        filesz: 0,
        flags: 0,
    };

    pub fn readable(&self) -> bool {
        self.flags & PF_R != 0
    }

    pub fn writable(&self) -> bool {
        self.flags & PF_W != 0
    }

    pub fn executable(&self) -> bool {
        self.flags & PF_X != 0
    }

    /// Intervalo virtual `[início, fim)` do segmento arredondado à página: o que
    /// o carregador mapeia. Satura em vez de estourar se `page_size` não for o
    /// dos `Limits` com que a imagem foi validada.
    pub fn page_range(&self, page_size: u64) -> (u64, u64) {
        let mask = page_size.wrapping_sub(1);
        let end = self.vaddr.saturating_add(self.memsz);
        (self.vaddr & !mask, end.saturating_add(mask) & !mask)
    }

    /// Onde `vaddr` cai dentro da sua página: quantos bytes da primeira página
    /// vêm antes do primeiro byte do segmento.
    pub fn page_offset(&self, page_size: u64) -> u64 {
        self.vaddr & page_size.wrapping_sub(1)
    }

    /// Os `p_filesz` bytes do segmento em `file`, que valem a partir de `vaddr`
    /// (não do início da página). O chamador zera o começo da primeira página
    /// antes de `vaddr` e a cauda de `filesz` até `memsz`.
    pub fn file_bytes<'a>(&self, file: &'a [u8]) -> Option<&'a [u8]> {
        let start = usize::try_from(self.offset).ok()?;
        let end = usize::try_from(self.offset.checked_add(self.filesz)?).ok()?;
        file.get(start..end)
    }
}

/// O que o carregador precisa de um executável válido.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Image {
    /// Endereço virtual da primeira instrução.
    pub entry: u64,
    loads: [Segment; MAX_LOADS],
    load_count: usize,
    /// Endereço virtual da tabela de program headers, se ela cai dentro de um
    /// `PT_LOAD` (o `AT_PHDR` do auxv).
    pub phdr: Option<u64>,
    /// `e_phnum` (o `AT_PHNUM`).
    pub phnum: u16,
}

impl Image {
    /// Os `PT_LOAD`, em ordem crescente de endereço.
    pub fn loads(&self) -> &[Segment] {
        self.loads.get(..self.load_count).unwrap_or_default()
    }
}

/// Valida `file` contra `limits` e devolve a imagem.
pub fn parse(file: &[u8], limits: &Limits) -> Result<Image, Error> {
    if !limits.page_size.is_power_of_two() || limits.user_min > limits.user_top {
        return Err(Error::BadLimits);
    }
    let header = Header::read(file, limits.machine)?;
    let table = header.table(file.len())?;
    let mut image = Image {
        entry: header.entry,
        loads: [Segment::EMPTY; MAX_LOADS],
        load_count: 0,
        phdr: None,
        phnum: header.phnum,
    };
    let mut total = 0u64;
    let mut previous = (0u64, 0u64);
    let mut entry_ok = false;
    for index in 0..usize::from(header.phnum) {
        let phdr = Phdr::read(file, table + index * PHDR_LEN)?;
        match phdr.kind {
            PT_INTERP => return Err(Error::Interp),
            PT_TLS => return Err(Error::Tls),
            PT_DYNAMIC => return Err(Error::Dynamic),
            PT_LOAD => {}
            _ => continue,
        }
        let (start, end) = check_load(&phdr, file.len(), limits)?;
        if image.load_count == MAX_LOADS {
            return Err(Error::TooManyLoads);
        }
        if image.load_count > 0 {
            if phdr.vaddr < previous.0 {
                return Err(Error::Unsorted);
            }
            if start < previous.1 {
                return Err(Error::Overlap);
            }
        }
        total = total.checked_add(phdr.memsz).ok_or(Error::TooLarge)?;
        if total > limits.max_loads_bytes {
            return Err(Error::TooLarge);
        }
        previous = (phdr.vaddr, end);
        let segment = Segment {
            vaddr: phdr.vaddr,
            memsz: phdr.memsz,
            offset: phdr.offset,
            filesz: phdr.filesz,
            flags: phdr.flags & (PF_R | PF_W | PF_X),
        };
        entry_ok |= segment.executable()
            && header.entry >= segment.vaddr
            && header.entry - segment.vaddr < segment.filesz;
        if image.phdr.is_none() {
            image.phdr = phdr_address(&segment, &header);
        }
        image.loads[image.load_count] = segment;
        image.load_count += 1;
    }
    if !entry_ok {
        return Err(Error::EntryNotExecutable);
    }
    Ok(image)
}

/// Confere um `PT_LOAD` sozinho e devolve a faixa de páginas `[início, fim)`.
fn check_load(phdr: &Phdr, file_len: usize, limits: &Limits) -> Result<(u64, u64), Error> {
    let mask = limits.page_size - 1;
    if phdr.memsz == 0 {
        return Err(Error::EmptyLoad);
    }
    if phdr.filesz > phdr.memsz {
        return Err(Error::FileszGreaterThanMemsz);
    }
    if phdr.align > 1 && !phdr.align.is_power_of_two() {
        return Err(Error::BadAlign);
    }
    let file_end = phdr
        .offset
        .checked_add(phdr.filesz)
        .ok_or(Error::SegmentOutOfFile)?;
    if file_end > u64::try_from(file_len).unwrap_or(u64::MAX) {
        return Err(Error::SegmentOutOfFile);
    }
    let mem_end = phdr
        .vaddr
        .checked_add(phdr.memsz)
        .ok_or(Error::VaddrOverflow)?;
    let end = mem_end.checked_add(mask).ok_or(Error::VaddrOverflow)? & !mask;
    let start = phdr.vaddr & !mask;
    if start < limits.user_min || end > limits.user_top {
        return Err(Error::OutOfUserRange);
    }
    if phdr.vaddr & mask != phdr.offset & mask {
        return Err(Error::Misaligned);
    }
    if phdr.flags & PF_W != 0 && phdr.flags & PF_X != 0 {
        return Err(Error::WritableAndExecutable);
    }
    if phdr.flags & PF_R == 0 {
        return Err(Error::NotReadable);
    }
    Ok((start, end))
}

/// Endereço da tabela de program headers na memória, se ela cabe inteira no
/// conteúdo de arquivo de `segment`.
fn phdr_address(segment: &Segment, header: &Header) -> Option<u64> {
    let table_end = header.phoff.checked_add(table_bytes(header.phnum))?;
    let file_end = segment.offset.checked_add(segment.filesz)?;
    if header.phoff < segment.offset || table_end > file_end {
        return None;
    }
    segment.vaddr.checked_add(header.phoff - segment.offset)
}

fn table_bytes(phnum: u16) -> u64 {
    u64::from(phnum) * PHDR_LEN as u64
}

struct Header {
    entry: u64,
    phoff: u64,
    phnum: u16,
}

impl Header {
    fn read(file: &[u8], machine: u16) -> Result<Self, Error> {
        let ehdr = file.get(..EHDR_LEN).ok_or(Error::Truncated)?;
        if ehdr.get(..4) != Some(b"\x7fELF") {
            return Err(Error::BadMagic);
        }
        if ehdr.get(4) != Some(&2) {
            return Err(Error::BadClass);
        }
        if ehdr.get(5) != Some(&1) {
            return Err(Error::BadData);
        }
        if ehdr.get(6) != Some(&1) {
            return Err(Error::BadVersion);
        }
        if ehdr.get(7) != Some(&0) {
            return Err(Error::BadOsAbi);
        }
        if read_u16(ehdr, 16)? != ET_EXEC {
            return Err(Error::BadType);
        }
        if read_u16(ehdr, 18)? != machine {
            return Err(Error::BadMachine);
        }
        if read_u32(ehdr, 20)? != 1 {
            return Err(Error::BadVersion);
        }
        if usize::from(read_u16(ehdr, 54)?) != PHDR_LEN {
            return Err(Error::BadPhentsize);
        }
        Ok(Self {
            entry: read_u64(ehdr, 24)?,
            phoff: read_u64(ehdr, 32)?,
            phnum: read_u16(ehdr, 56)?,
        })
    }

    /// Offset da tabela no arquivo, depois de conferir tamanho e limites.
    fn table(&self, file_len: usize) -> Result<usize, Error> {
        let size = table_bytes(self.phnum);
        if size == 0 || size > MAX_PHDR_TABLE {
            return Err(Error::BadPhdrTableSize);
        }
        let end = self
            .phoff
            .checked_add(size)
            .ok_or(Error::PhdrTableOutOfFile)?;
        if end > u64::try_from(file_len).unwrap_or(u64::MAX) {
            return Err(Error::PhdrTableOutOfFile);
        }
        usize::try_from(self.phoff).map_err(|_| Error::PhdrTableOutOfFile)
    }
}

struct Phdr {
    kind: u32,
    flags: u32,
    offset: u64,
    vaddr: u64,
    filesz: u64,
    memsz: u64,
    align: u64,
}

impl Phdr {
    fn read(file: &[u8], at: usize) -> Result<Self, Error> {
        let end = at.checked_add(PHDR_LEN).ok_or(Error::PhdrTableOutOfFile)?;
        let raw = file.get(at..end).ok_or(Error::PhdrTableOutOfFile)?;
        Ok(Self {
            kind: read_u32(raw, 0)?,
            flags: read_u32(raw, 4)?,
            offset: read_u64(raw, 8)?,
            vaddr: read_u64(raw, 16)?,
            filesz: read_u64(raw, 32)?,
            memsz: read_u64(raw, 40)?,
            align: read_u64(raw, 48)?,
        })
    }
}

fn read_array<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N], Error> {
    let end = at.checked_add(N).ok_or(Error::Truncated)?;
    bytes
        .get(at..end)
        .and_then(|slice| slice.try_into().ok())
        .ok_or(Error::Truncated)
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, Error> {
    read_array(bytes, at).map(u16::from_le_bytes)
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, Error> {
    read_array(bytes, at).map(u32::from_le_bytes)
}

fn read_u64(bytes: &[u8], at: usize) -> Result<u64, Error> {
    read_array(bytes, at).map(u64::from_le_bytes)
}
