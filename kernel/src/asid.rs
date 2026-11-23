//! As etiquetas de TLB dos espaços de usuário (SDD 17, "PCID e ASID com várias CPUs"): o gerador
//! global atrás de um lock, a regra de geração na troca e a migração pelo `tlb_gen` do espaço.
//! Sem etiqueta (sem PCID/ASID), a troca é a de antes da 4e.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use rustos_hal::{MAX_CPUS, TagFlush, TlbTag};
use rustos_mm::asid::{Asid, AsidAllocator};

use crate::sync::{IrqLock, bump};
use crate::uspace::Shared;

/// O gerador: 1024 palavras cobrem o ASID de 16 bits (o PCID de 12 usa 64).
static ALLOC: IrqLock<Option<AsidAllocator<1024, MAX_CPUS>>> = IrqLock::new(None);
/// As etiquetas valem nesta máquina (decidido no boot, antes do primeiro espaço de usuário).
/// A CPU anuncia PCID (o x86_64 o diz no `user_init`; no aarch64 não se usa).
static HARDWARE_PCID: AtomicBool = AtomicBool::new(false);

/// Anota se a CPU anuncia PCID, para a linha `tlb tags:` dizer `absent` ou `untested`.
#[cfg(target_arch = "x86_64")]
pub fn note_hardware(pcid: bool) {
    HARDWARE_PCID.store(pcid, Ordering::Relaxed);
}
static ON: AtomicBool = AtomicBool::new(false);
/// A largura em uso (o que o gerador recebeu: no modo de teste, 4 bits).
static BITS: AtomicU32 = AtomicU32::new(0);
/// A largura que o hardware anuncia (12 no PCID, 8 ou 16 no ASID), para a linha `tlb tags:`.
static HARDWARE_BITS: AtomicU32 = AtomicU32::new(0);
/// Esvaziamentos de TLB que a regra mandou (tudo e só de uma etiqueta), para o autoteste.
static FLUSH_ALL: AtomicU64 = AtomicU64::new(0);
static FLUSH_SPACE: AtomicU64 = AtomicU64::new(0);

/// No modo de teste a largura é de 4 bits (15 etiquetas para 8 CPUs): o esgotamento acontece a cada
/// poucas trocas e o autoteste exercita a virada de geração o tempo todo.
#[cfg(feature = "selftest")]
const TEST_BITS: u32 = 4;

/// Liga as etiquetas com `bits` de largura (`None`: a máquina não tem PCID/ASID). Antes de qualquer
/// espaço de usuário. Devolve se valem.
pub fn init(bits: Option<u32>) -> bool {
    let Some(hardware) = bits else {
        return false;
    };
    HARDWARE_BITS.store(hardware, Ordering::Relaxed);
    #[cfg(feature = "selftest")]
    let bits = hardware.min(TEST_BITS);
    #[cfg(not(feature = "selftest"))]
    let bits = hardware;
    let Ok(alloc) = AsidAllocator::new(bits, 1) else {
        return false;
    };
    *ALLOC.lock() = Some(alloc);
    BITS.store(bits, Ordering::Relaxed);
    ON.store(true, Ordering::Release);
    true
}

/// As etiquetas valem nesta máquina.
pub fn enabled() -> bool {
    ON.load(Ordering::Acquire)
}

/// O que a troca para o espaço `shared` faz na CPU `cpu` (zero: a raiz do kernel, etiqueta 0). Toma o
/// gerador e fica com ele só o tempo de decidir; quem carrega a raiz é o `arch/`.
pub fn for_switch(cpu: usize, shared: usize) -> TlbTag {
    if !enabled() {
        return TlbTag::Untagged;
    }
    if shared == 0 {
        return TlbTag::Tagged {
            index: 0,
            flush: TagFlush::Keep,
        };
    }
    // SAFETY: o endereço é o do `Shared` de um `UserSpace` ligado por `bind_user`; vive até
    // `unbind_user` e não se move enquanto a tarefa o roda (o mesmo contrato de `mark_active`).
    let shared = unsafe { &*core::ptr::with_exposed_provenance::<Shared>(shared) };
    let mut guard = ALLOC.lock();
    let alloc = guard
        .as_mut()
        .expect("etiquetas ligadas sem gerador: init não rodou");
    let current = Asid::from_raw(shared.asid.load(Ordering::SeqCst));
    let switch = alloc
        .switch(cpu, current)
        .expect("o gerador recusou a troca: a CPU ou a geração estão fora do alcance");
    let index = alloc.index(switch.asid);
    shared.asid.store(switch.asid.raw(), Ordering::SeqCst);
    // A geração lida aqui é a que a CPU vê: se algum `unmap` subiu depois de a CPU entrar na máscara,
    // a diferença manda esvaziar só esta etiqueta.
    let now = shared.tlb_gen.load(Ordering::SeqCst);
    let seen = &shared.seen[cpu];
    let flush = if switch.flush {
        TagFlush::All
    } else if now > seen.load(Ordering::Relaxed) {
        TagFlush::Space
    } else {
        TagFlush::Keep
    };
    match flush {
        TagFlush::All => {
            bump(&FLUSH_ALL, 1);
            seen.store(now, Ordering::Relaxed);
        }
        TagFlush::Space => {
            bump(&FLUSH_SPACE, 1);
            seen.store(now, Ordering::Relaxed);
        }
        TagFlush::Keep => {}
    }
    TlbTag::Tagged { index, flush }
}

/// Gerações abertas, esvaziamentos de tudo e de uma etiqueta até aqui (o autoteste e o `bench`).
#[cfg(feature = "selftest")]
pub fn stats() -> (u64, u64, u64) {
    let generation = ALLOC.lock().as_ref().map_or(0, AsidAllocator::generation);
    (
        generation,
        FLUSH_ALL.load(Ordering::Relaxed),
        FLUSH_SPACE.load(Ordering::Relaxed),
    )
}

/// A largura em uso, em bits (zero sem etiquetas).
#[cfg(feature = "selftest")]
pub fn bits() -> u32 {
    BITS.load(Ordering::Relaxed)
}

/// O que a linha `tlb tags:` diz: a largura do hardware e, no modo de teste, a largura de teste.
pub struct Label;

impl core::fmt::Display for Label {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let on = enabled();
        let hardware = HARDWARE_BITS.load(Ordering::Relaxed);
        if cfg!(target_arch = "x86_64") {
            if on {
                f.write_str("pcid on")?;
            } else if HARDWARE_PCID.load(Ordering::Relaxed) {
                f.write_str("pcid off (untested)")?;
            } else {
                f.write_str("pcid off (absent)")?;
            }
        } else if on {
            write!(f, "asid {hardware}-bit")?;
        } else {
            f.write_str("asid off")?;
        }
        #[cfg(feature = "selftest")]
        if on && BITS.load(Ordering::Relaxed) != hardware {
            write!(f, " (test tags {}-bit)", BITS.load(Ordering::Relaxed))?;
        }
        Ok(())
    }
}

/// A linha `tlb tags:` (ver `Label`).
pub fn label() -> Label {
    Label
}
