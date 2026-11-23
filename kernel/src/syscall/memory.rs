//! As chamadas de memória da ABI nativa (SDD 17, fatia 4c, parte 3): `vmo_create`, `vmar_map` e
//! `vmar_unmap`. O VMO é teto das permissões de quem o mapeia (os direitos do handle), o W^X vale por
//! mapeamento, e a decisão 9 (reserva estrita) dá `NoMemory` aqui, e não a morte na falta de página.

use alloc::sync::Arc;

use rustos_hal::PAGE_SIZE;
use rustos_hal::abi::{Rights, Status, VMO_MAX_BYTES, vmar_flags};
use rustos_mm::vmar::Perms;
use rustos_object::rights::required;
use rustos_object::{ObjectKind, handle_from_raw};

use super::handles::{Outcome, current, u32_arg};
use crate::object::Obj;
use crate::process::{MapRequest, Process, VmarObj};
use crate::vmo::Vmo;

/// `vmo_create(size, flags)`: um VMO anônimo e preguiçoso de `size` bytes (múltiplo de página, de 1
/// página a `VMO_MAX_BYTES`), prometido contra os frames livres e cobrado ao processo.
pub fn vmo_create(size: u64, flags: u64) -> Outcome {
    if flags != 0 || size == 0 || !size.is_multiple_of(PAGE_SIZE) || size > VMO_MAX_BYTES {
        return Err(Status::InvalidArgs);
    }
    let frames = crate::process::frames().ok_or(Status::NotSupported)?;
    current(|p| {
        let slot = p.table().lock().reserve()?;
        match Vmo::anonymous(frames, p.quota(), size / PAGE_SIZE) {
            Ok(vmo) => Ok(p
                .table()
                .lock()
                .commit(slot, Obj::Vmo(vmo), Rights::VMO)
                .raw()),
            Err(status) => {
                p.table().lock().cancel(slot);
                Err(status)
            }
        }
    })
}

/// O VMAR e, se pedido, o VMO que os handles dão, com os direitos que o mapeamento exige.
fn lookup(
    p: &Process,
    vmar: u64,
    vmo: Option<(u64, Rights)>,
    vmar_rights: Rights,
) -> Result<(Arc<VmarObj>, Option<Arc<Vmo>>), Status> {
    let table = p.table().lock();
    let granted = table.get_of(handle_from_raw(vmar)?, ObjectKind::Vmar, vmar_rights)?;
    let Obj::Vmar(target) = granted.object else {
        return Err(Status::WrongType);
    };
    let target = Arc::clone(target);
    let object = match vmo {
        Some((raw, rights)) => {
            let granted = table.get_of(handle_from_raw(raw)?, ObjectKind::Vmo, rights)?;
            let Obj::Vmo(object) = granted.object else {
                return Err(Status::WrongType);
            };
            Some(Arc::clone(object))
        }
        None => None,
    };
    Ok((target, object))
}

/// As flags de `vmar_map` como permissões da região.
fn perms_of(flags: u32) -> Perms {
    let mut perms = Perms::NONE;
    if flags & vmar_flags::READ != 0 {
        perms = perms.union(Perms::READ);
    }
    if flags & vmar_flags::WRITE != 0 {
        perms = perms.union(Perms::WRITE);
    }
    if flags & vmar_flags::EXECUTE != 0 {
        perms = perms.union(Perms::EXECUTE);
    }
    perms
}

/// `vmar_map(vmar, vmo, vmo_off, len, flags, vmar_off)`: mapeia `len` bytes do VMO a partir de
/// `vmo_off`. Os dois handles têm `MAP` e cada permissão pedida (o direito de VMO e de VMAR é o teto);
/// W+X é `InvalidArgs`. Com `FIXED` o endereço é `vmar_off` e sobrepor é `InvalidArgs`; sem ele o
/// kernel escolhe a partir do `mmap_base` sorteado e `vmar_off` tem de ser 0. Devolve o endereço.
pub fn vmar_map(args: &[u64; 6]) -> Outcome {
    let [vmar, vmo, vmo_off, len, flags, vmar_off] = *args;
    let flags = u32_arg(flags)
        .ok()
        .and_then(vmar_flags::validate)
        .ok_or(Status::InvalidArgs)?;
    let needed = required::vmar_map(flags);
    let (target, object) = current(|p| lookup(p, vmar, Some((vmo, needed)), needed))?;
    let object = object.ok_or(Status::BadHandle)?;
    let fixed = flags & vmar_flags::FIXED != 0;
    if len == 0
        || !len.is_multiple_of(PAGE_SIZE)
        || !vmo_off.is_multiple_of(PAGE_SIZE)
        || !fixed && vmar_off != 0
    {
        return Err(Status::InvalidArgs);
    }
    let end = vmo_off.checked_add(len).ok_or(Status::OutOfRange)?;
    if end > object.pages() * PAGE_SIZE {
        return Err(Status::OutOfRange);
    }
    let request = MapRequest {
        offset_pages: vmo_off / PAGE_SIZE,
        pages: len / PAGE_SIZE,
        perms: perms_of(flags),
        fixed: fixed.then_some(vmar_off),
    };
    target.process().with_mem(|mem| mem.map(&object, request))?
}

/// `vmar_unmap(vmar, addr, len)`: solta a faixa (inteira ou em parte de uma região) com *shootdown*.
/// Faixa sem região é sucesso. Partir uma região sem vaga na árvore é `NoMemory`, e nada muda.
pub fn vmar_unmap(vmar: u64, addr: u64, len: u64) -> Outcome {
    let (target, _) = current(|p| lookup(p, vmar, None, required::VMAR_UNMAP))?;
    target
        .process()
        .with_mem(|mem| mem.unmap(addr, len))?
        .map(|()| 0)
}
