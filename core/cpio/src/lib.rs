//! Leitor de cpio `newc` sobre `&[u8]`, sem alocação: o initramfs do boot entra
//! aqui e só arquivo regular sai (SDD 03, "Formatos de entrada").

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests;

/// Tamanho do cabeçalho: o magic de 6 bytes e 13 campos de 8 dígitos hexadecimais.
pub const HEADER_LEN: usize = 6 + 13 * 8;
/// Maior `c_namesize` aceito (`PATH_MAX`).
pub const MAX_NAME_SIZE: usize = 4096;
/// Nome do registro que fecha o arquivo.
pub const TRAILER: &[u8] = b"TRAILER!!!";

const MAGIC: &[u8; 6] = b"070701";
const MAGIC_CHECKSUM: &[u8; 6] = b"070702";
const S_IFMT: u32 = 0o170_000;
const S_IFREG: u32 = 0o100_000;
const S_IFDIR: u32 = 0o040_000;
const S_IFLNK: u32 = 0o120_000;

/// Por que o arquivo foi recusado. Cada variante é uma regra do SDD 03.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// O buffer acaba antes do fim de um cabeçalho, nome ou dado.
    Truncated,
    /// O arquivo acaba sem o registro `TRAILER!!!`.
    MissingTrailer,
    /// O magic não é `070701`.
    BadMagic,
    /// O magic é `070702` (com soma de verificação), que não é aceito.
    ChecksumFormat,
    /// Um dos 13 campos tem caractere que não é dígito hexadecimal.
    BadHex,
    /// `c_namesize` é zero ou passa de [`MAX_NAME_SIZE`].
    BadNameSize,
    /// O nome não tem NUL dentro de `c_namesize`, ou é vazio.
    BadName,
    /// O `TRAILER!!!` tem `c_filesize` diferente de zero.
    BadTrailer,
    /// Diretório com dado.
    DirectoryWithData,
    /// Link simbólico.
    Symlink,
    /// Dispositivo, FIFO, socket ou tipo desconhecido.
    SpecialFile,
    /// Arquivo regular com `c_nlink` diferente de 1 (hard link ou contagem zero).
    LinkCount,
}

/// Um arquivo regular do cpio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry<'a> {
    /// Nome, sem o NUL; comparado byte a byte, sem tirar `./`.
    pub name: &'a [u8],
    /// Conteúdo, emprestado do buffer.
    pub data: &'a [u8],
    /// `c_mode` inteiro: tipo e permissões.
    pub mode: u32,
}

/// Iterador sobre os arquivos regulares. Pula diretório sem dado; a primeira
/// recusa vira `Err` e encerra a iteração.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    archive: &'a [u8],
    offset: usize,
    done: bool,
}

struct Header {
    mode: u32,
    nlink: u32,
    filesize: usize,
    namesize: usize,
}

impl<'a> Reader<'a> {
    /// Começa no byte 0 de `archive`, onde o primeiro cabeçalho tem de estar.
    pub fn new(archive: &'a [u8]) -> Self {
        Self {
            archive,
            offset: 0,
            done: false,
        }
    }

    fn next_entry(&mut self) -> Result<Option<Entry<'a>>, Error> {
        loop {
            let rest = self.archive.get(self.offset..).unwrap_or_default();
            if rest.is_empty() {
                return Err(Error::MissingTrailer);
            }
            let header = parse_header(rest)?;
            let name_end = HEADER_LEN
                .checked_add(header.namesize)
                .ok_or(Error::Truncated)?;
            let field = rest.get(HEADER_LEN..name_end).ok_or(Error::Truncated)?;
            let name_len = field
                .iter()
                .position(|&byte| byte == 0)
                .ok_or(Error::BadName)?;
            let name = field.get(..name_len).ok_or(Error::BadName)?;
            if name == TRAILER {
                return if header.filesize == 0 {
                    Ok(None)
                } else {
                    Err(Error::BadTrailer)
                };
            }
            if name.is_empty() {
                return Err(Error::BadName);
            }
            let data_start = align4(name_end)?;
            let data_end = data_start
                .checked_add(header.filesize)
                .ok_or(Error::Truncated)?;
            let data = rest.get(data_start..data_end).ok_or(Error::Truncated)?;
            self.offset = self
                .offset
                .checked_add(align4(data_end)?)
                .ok_or(Error::Truncated)?;
            match header.mode & S_IFMT {
                S_IFREG if header.nlink == 1 => {
                    return Ok(Some(Entry {
                        name,
                        data,
                        mode: header.mode,
                    }));
                }
                S_IFREG => return Err(Error::LinkCount),
                S_IFDIR if data.is_empty() => {}
                S_IFDIR => return Err(Error::DirectoryWithData),
                S_IFLNK => return Err(Error::Symlink),
                _ => return Err(Error::SpecialFile),
            }
        }
    }
}

impl<'a> Iterator for Reader<'a> {
    type Item = Result<Entry<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let item = self.next_entry().transpose();
        if !matches!(item, Some(Ok(_))) {
            self.done = true;
        }
        item
    }
}

/// Acha o arquivo regular `name`. Percorre o arquivo inteiro, então a validade
/// de tudo, `TRAILER!!!` incluído, vale mesmo com o achado no começo; nome
/// repetido fica com a última ocorrência.
pub fn find<'a>(archive: &'a [u8], name: &[u8]) -> Result<Option<Entry<'a>>, Error> {
    let mut found = None;
    for entry in Reader::new(archive) {
        let entry = entry?;
        if entry.name == name {
            found = Some(entry);
        }
    }
    Ok(found)
}

fn align4(offset: usize) -> Result<usize, Error> {
    offset
        .checked_add(3)
        .map(|value| value & !3)
        .ok_or(Error::Truncated)
}

fn parse_header(rest: &[u8]) -> Result<Header, Error> {
    let header = rest.get(..HEADER_LEN).ok_or(Error::Truncated)?;
    let magic = header.get(..6).ok_or(Error::Truncated)?;
    if magic == MAGIC_CHECKSUM {
        return Err(Error::ChecksumFormat);
    }
    if magic != MAGIC {
        return Err(Error::BadMagic);
    }
    let mut fields = [0u32; 13];
    for (index, slot) in fields.iter_mut().enumerate() {
        let start = 6 + index * 8;
        let digits = header.get(start..start + 8).ok_or(Error::Truncated)?;
        *slot = parse_hex(digits)?;
    }
    // Ordem do `newc`: ino, mode, uid, gid, nlink, mtime, filesize, maj, min,
    // rmaj, rmin, namesize, chksum.
    let namesize = usize::try_from(fields[11]).map_err(|_| Error::BadNameSize)?;
    if namesize == 0 || namesize > MAX_NAME_SIZE {
        return Err(Error::BadNameSize);
    }
    Ok(Header {
        mode: fields[1],
        nlink: fields[4],
        filesize: usize::try_from(fields[6]).map_err(|_| Error::Truncated)?,
        namesize,
    })
}

fn parse_hex(digits: &[u8]) -> Result<u32, Error> {
    let mut value = 0u32;
    for &digit in digits {
        let nibble = match digit {
            b'0'..=b'9' => digit - b'0',
            b'a'..=b'f' => digit - b'a' + 10,
            b'A'..=b'F' => digit - b'A' + 10,
            _ => return Err(Error::BadHex),
        };
        value = (value << 4) | u32::from(nibble);
    }
    Ok(value)
}
