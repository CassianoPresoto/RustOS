//! A mensagem de boot (SDD 17, "O canal de boot"): o programa lê, da ponta que recebeu em `rdi`/`x0`,
//! os handles que o pai lhe deu, na ordem dos `boot_kind` da mensagem.

use rustos_hal::abi::{Handle, boot_kind};

/// Os handles da mensagem de boot que o programa reconheceu. O que o pai não mandou fica `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Boot {
    pub process: Option<Handle>,
    pub vmar_root: Option<Handle>,
    pub console: Option<Handle>,
    pub initramfs: Option<Handle>,
}

impl Boot {
    /// Distribui os handles pelos `kinds` da mensagem; um tipo que não conhece é ignorado. `None` se
    /// `handles` não tem um handle por tipo.
    pub fn from_items(kinds: &[u32], handles: &[Handle]) -> Option<Self> {
        if kinds.len() != handles.len() {
            return None;
        }
        let mut boot = Self::default();
        for (&kind, &handle) in kinds.iter().zip(handles) {
            let slot = match kind {
                boot_kind::PROCESS => &mut boot.process,
                boot_kind::VMAR_ROOT => &mut boot.vmar_root,
                boot_kind::CONSOLE => &mut boot.console,
                boot_kind::INITRAMFS => &mut boot.initramfs,
                _ => continue,
            };
            *slot = Some(handle);
        }
        Some(boot)
    }
}

/// Lê a mensagem de boot de `channel`. `InvalidArgs` se a mensagem não é a do contrato.
#[cfg(target_os = "none")]
pub fn read(channel: Handle) -> Result<Boot, rustos_hal::abi::Status> {
    use rustos_hal::abi::{CHANNEL_MAX_BYTES, CHANNEL_MAX_HANDLES, Status, parse_boot_message};
    let mut bytes = [0_u8; CHANNEL_MAX_BYTES];
    let mut handles = [Handle::INVALID; CHANNEL_MAX_HANDLES];
    let actual = crate::sys::channel_read(channel, &mut bytes, &mut handles)
        .map_err(|error| error.status)?;
    let (len, count) = (
        usize::try_from(actual.bytes).map_err(|_| Status::InvalidArgs)?,
        usize::try_from(actual.handles).map_err(|_| Status::InvalidArgs)?,
    );
    let mut kinds = [0_u32; CHANNEL_MAX_HANDLES];
    let items = bytes
        .get(..len)
        .and_then(|message| parse_boot_message(message, &mut kinds))
        .ok_or(Status::InvalidArgs)?;
    Boot::from_items(&kinds[..items], &handles[..count]).ok_or(Status::InvalidArgs)
}

/// Lê a mensagem de boot do canal que `env` recebeu e passa `println!` a escrever pelo console dela.
#[cfg(target_os = "none")]
pub fn attach_console(env: &crate::Env) -> Option<Boot> {
    let boot = read(env.boot_channel()?).ok()?;
    crate::set_console(boot.console?);
    Some(boot)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(raw: u64) -> Handle {
        Handle::from_raw(raw).unwrap()
    }

    #[test]
    fn handles_follow_the_kinds_of_the_message_and_missing_ones_stay_none() {
        let boot = Boot::from_items(
            &[boot_kind::PROCESS, boot_kind::CONSOLE],
            &[handle(1), handle(2)],
        )
        .unwrap();
        assert_eq!(boot.process, Some(handle(1)));
        assert_eq!(boot.console, Some(handle(2)));
        assert_eq!((boot.vmar_root, boot.initramfs), (None, None));
    }

    #[test]
    fn a_kind_the_runtime_does_not_know_is_ignored_and_a_count_mismatch_is_refused() {
        let boot = Boot::from_items(&[99, boot_kind::CONSOLE], &[handle(5), handle(6)]).unwrap();
        assert_eq!(boot.console, Some(handle(6)));
        assert_eq!(Boot::from_items(&[1, 3], &[handle(1)]), None);
    }
}
