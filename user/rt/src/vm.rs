//! Memória do próprio processo pela ABI: mapear um VMO em si mesmo para ler ou preencher, e soltar. O
//! `unsafe` das fatias mora aqui: o mapeamento é de um VMO que só este código usa enquanto existe.

use core::slice;

use rustos_hal::PAGE_SIZE;
use rustos_hal::abi::{Handle, Status, vmar_flags};

use crate::sys;

/// Um VMO mapeado neste processo, `[addr, addr + len)`. Solto no `Drop`.
#[derive(Debug)]
pub struct Mapping {
    vmar: Handle,
    addr: u64,
    len: usize,
}

impl Mapping {
    /// Mapeia `len` bytes de `vmo` a partir de `offset` no VMAR `vmar` (o raiz do processo), com
    /// `flags` de `vmar_flags` (`READ`, e `WRITE` para preencher).
    pub fn new(
        vmar: Handle,
        vmo: Handle,
        offset: u64,
        len: usize,
        flags: u32,
    ) -> Result<Self, Status> {
        let addr = sys::vmar_map(vmar, vmo, offset, len as u64, flags, 0)?;
        Ok(Self { vmar, addr, len })
    }

    /// O endereço onde o kernel pôs o mapeamento.
    pub const fn addr(&self) -> u64 {
        self.addr
    }

    /// Os bytes do mapeamento, para ler.
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: `[addr, addr + len)` é um mapeamento deste processo com `READ`, que `Drop` só solta
        // depois de o empréstimo acabar; o VMO é de quem o mapeou.
        unsafe { slice::from_raw_parts(core::ptr::with_exposed_provenance(self.addr as usize), self.len) }
    }

    /// Os bytes do mapeamento, para preencher. Só vale em mapeamento com `WRITE`.
    pub fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: como `bytes`, e `&mut self` impede outra visão do mesmo mapeamento.
        unsafe {
            slice::from_raw_parts_mut(
                core::ptr::with_exposed_provenance_mut(self.addr as usize),
                self.len,
            )
        }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        let _ = sys::vmar_unmap(self.vmar, self.addr, (self.len as u64).next_multiple_of(PAGE_SIZE));
    }
}

/// Um VMO novo de `len` bytes (arredondado à página) e a leitura e escrita dele neste processo.
pub fn scratch(vmar: Handle, len: usize) -> Result<(Handle, Mapping), Status> {
    let size = (len as u64).next_multiple_of(PAGE_SIZE);
    let vmo = sys::vmo_create(size)?;
    let mapping = Mapping::new(
        vmar,
        vmo,
        0,
        size as usize,
        vmar_flags::READ | vmar_flags::WRITE,
    );
    match mapping {
        Ok(mapping) => Ok((vmo, mapping)),
        Err(status) => {
            let _ = sys::handle_close(vmo);
            Err(status)
        }
    }
}

/// O initramfs lido pelo VMO físico que o `init` recebeu: mapeado inteiro, só leitura, e percorrido
/// com `core/cpio`. A ABI não diz o tamanho de um VMO (DEBT-0140): `open` o acha por tentativa de
/// mapeamento, em páginas.
#[derive(Debug)]
pub struct Archive {
    map: Mapping,
}

impl Archive {
    /// Mapeia o VMO `vmo` no VMAR `vmar`. `OutOfRange` é o sinal de que o VMO acabou; outro erro
    /// aborta.
    pub fn open(vmar: Handle, vmo: Handle) -> Result<Self, Status> {
        let fits = |pages: u64| -> Result<bool, Status> {
            match Mapping::new(vmar, vmo, 0, (pages * PAGE_SIZE) as usize, vmar_flags::READ) {
                Ok(_) => Ok(true),
                Err(Status::OutOfRange) => Ok(false),
                Err(status) => Err(status),
            }
        };
        let (mut low, mut high) = (0_u64, 1_u64);
        while fits(high)? {
            low = high;
            high *= 2;
        }
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            if fits(middle)? {
                low = middle;
            } else {
                high = middle;
            }
        }
        if low == 0 {
            return Err(Status::InvalidArgs);
        }
        let map = Mapping::new(vmar, vmo, 0, (low * PAGE_SIZE) as usize, vmar_flags::READ)?;
        Ok(Self { map })
    }

    /// O arquivo regular `name` (a última ocorrência), ou `None`.
    pub fn file(&self, name: &[u8]) -> Option<&[u8]> {
        rustos_cpio::find(self.map.bytes(), name)
            .ok()
            .flatten()
            .map(|entry| entry.data)
    }
}
