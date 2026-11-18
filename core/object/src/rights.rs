//! As regras de direito: o que só diminui, e o que cada operação exige (SDD 17, "Direitos
//! iniciais" e "As chamadas"). Os bits são os de `hal::abi::Rights`.

use rustos_hal::abi::{Rights, Status, vmar_flags};

/// Os bits que o usuário passou, como direitos: bit reservado é `InvalidArgs`.
pub const fn parse(bits: u32) -> Result<Rights, Status> {
    match Rights::from_bits(bits) {
        Some(rights) => Ok(rights),
        None => Err(Status::InvalidArgs),
    }
}

/// O handle tem o que a operação exige, senão `AccessDenied`.
pub const fn require(have: Rights, required: Rights) -> Result<(), Status> {
    if have.contains(required) {
        Ok(())
    } else {
        Err(Status::AccessDenied)
    }
}

/// Direitos novos de `duplicate` e `replace`: só um subconjunto dos que o handle tem.
pub const fn reduce(have: Rights, wanted: Rights) -> Result<Rights, Status> {
    if wanted.is_subset_of(have) {
        Ok(wanted)
    } else {
        Err(Status::InvalidArgs)
    }
}

/// O direito que cada operação exige do handle que a autoriza. `NONE` é "nenhum" de verdade.
pub mod required {
    use super::{Rights, vmar_flags};

    pub const HANDLE_CLOSE: Rights = Rights::NONE;
    pub const HANDLE_DUPLICATE: Rights = Rights::DUPLICATE;
    /// Verificado no Zircon: `handle_replace` não pede direito.
    pub const HANDLE_REPLACE: Rights = Rights::NONE;
    pub const OBJECT_WAIT: Rights = Rights::WAIT;
    pub const DEBUG_WRITE: Rights = Rights::WRITE;
    pub const CHANNEL_WRITE: Rights = Rights::WRITE;
    pub const CHANNEL_READ: Rights = Rights::READ;
    /// Cada handle do array de `channel_write` e o `bootstrap` de `process_create`.
    pub const TRANSFER: Rights = Rights::TRANSFER;
    pub const PROCESS_CREATE_PARENT: Rights = Rights::MANAGE_PROCESS;
    pub const PROCESS_CREATE_BOOTSTRAP: Rights = Rights::TRANSFER;
    pub const PROCESS_CREATE_EVENTS: Rights = Rights::WRITE.union(Rights::TRANSFER);
    pub const THREAD_CREATE: Rights = Rights::MANAGE_THREAD;
    pub const PROCESS_KILL: Rights = Rights::DESTROY;
    pub const VMAR_UNMAP: Rights = Rights::MAP;

    /// `vmar_map`: o que o VMO e o teto do VMAR precisam ter, os mesmos bits dos dois lados.
    pub const fn vmar_map(flags: u32) -> Rights {
        vmar_flags::required_rights(flags)
    }
}
