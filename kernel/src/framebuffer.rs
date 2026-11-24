//! O framebuffer como VMO do usuário (SDD 18, caso 6): a descrição da tela, o VMO físico que o
//! `boot_kind::FRAMEBUFFER` entrega e a posse da tela. Enquanto um mapeamento do VMO existir, o console
//! não desenha (a serial segue); quando o último sai, o console redesenha a tela inteira no laço do shell.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use rustos_hal::abi::FramebufferFormat;
use rustos_hal::{FramebufferInfo, PAGE_SIZE, PhysAddr};

use crate::console;
use crate::frames::Frames;
use crate::sync::IrqLock;
use crate::vmo::Vmo;

/// Onde a tela está: o endereço virtual pelo qual o kernel a desenha e o físico, que o VMO usa.
#[derive(Clone, Copy, Debug)]
struct Layout {
    /// Só o autoteste lê pixels pelo endereço virtual.
    #[cfg_attr(
        not(feature = "selftest"),
        allow(dead_code, reason = "só o autoteste lê pixels")
    )]
    virt: u64,
    phys: PhysAddr,
    width: u32,
    height: u32,
    pitch: u32,
}

static LAYOUT: IrqLock<Option<Layout>> = IrqLock::new(None);
/// Mapeamentos vivos do VMO do framebuffer, em todos os processos.
static MAPPED: AtomicU32 = AtomicU32::new(0);
/// A última posse acabou e o console ainda não redesenhou a tela.
static REPAINT: AtomicBool = AtomicBool::new(false);
/// Vezes que o console quis desenhar e não pôde, porque a tela era de um programa.
static SKIPPED: AtomicU64 = AtomicU64::new(0);

/// Guarda a tela que o bootloader entregou. `direct_map_offset` leva o virtual ao físico. Só o formato
/// `XRGB8888` (vermelho nos bits 16 a 23, verde 8 a 15, azul 0 a 7, 8 bits cada) vira VMO: outro fica
/// sem o `boot_kind::FRAMEBUFFER`, e o console segue desenhando.
pub fn install(info: &FramebufferInfo, direct_map_offset: u64) {
    let xrgb = info.red.shift == 16
        && info.green.shift == 8
        && info.blue.shift == 0
        && [info.red, info.green, info.blue]
            .iter()
            .all(|channel| channel.size == 8);
    let Some(phys) = info.address.as_u64().checked_sub(direct_map_offset) else {
        return;
    };
    if xrgb {
        *LAYOUT.lock() = Some(Layout {
            virt: info.address.as_u64(),
            phys: PhysAddr::new(phys),
            width: info.width,
            height: info.height,
            pitch: info.pitch,
        });
    }
}

/// O VMO físico da tela e o formato que o `boot_kind::FRAMEBUFFER` leva; `None` sem tela XRGB.
pub fn boot_vmo(frames: Frames) -> Option<(Arc<Vmo>, FramebufferFormat)> {
    let layout = (*LAYOUT.lock())?;
    let bytes = u64::from(layout.pitch) * u64::from(layout.height);
    let vmo = Vmo::framebuffer(frames, layout.phys, bytes.div_ceil(PAGE_SIZE));
    let format = FramebufferFormat {
        width: layout.width,
        height: layout.height,
        stride: layout.pitch,
        format: FramebufferFormat::XRGB8888,
    };
    Some((vmo, format))
}

/// Um mapeamento do VMO do framebuffer nasce. Antes de voltar, espera o console terminar a
/// renderização que já começou: a partir daí, ele não desenha mais na tela.
pub fn hold() {
    MAPPED.fetch_add(1, Ordering::AcqRel);
    console::settle();
}

/// Um mapeamento do VMO do framebuffer saiu. Quando é o último, o console redesenha no laço do shell.
/// Sem lock e sem console: a morte do processo chama isto com IRQ mascarada.
pub fn release() {
    if MAPPED.fetch_sub(1, Ordering::AcqRel) == 1 {
        REPAINT.store(true, Ordering::Release);
    }
}

/// Se um programa segura a tela agora.
pub fn held() -> bool {
    MAPPED.load(Ordering::Acquire) > 0
}

/// Se o console deve redesenhar a tela inteira: a posse acabou desde a última vez. Consome a marca.
pub fn take_repaint() -> bool {
    !held() && REPAINT.swap(false, Ordering::AcqRel)
}

/// O console quis desenhar com a tela de um programa.
pub fn note_skipped() {
    SKIPPED.fetch_add(1, Ordering::Relaxed);
}

/// Quantas vezes o console deixou de desenhar por posse, desde o boot.
#[cfg(feature = "selftest")]
pub fn skipped() -> u64 {
    SKIPPED.load(Ordering::Relaxed)
}

/// Um pixel XRGB8888 da tela, em (`x`, `y`), lido pelo kernel; `None` fora dela ou sem tela.
#[cfg(feature = "selftest")]
pub fn pixel(x: u32, y: u32) -> Option<u32> {
    let layout = (*LAYOUT.lock())?;
    if x >= layout.width || y >= layout.height {
        return None;
    }
    let at = u64::from(y) * u64::from(layout.pitch) + u64::from(x) * 4;
    let address = usize::try_from(layout.virt + at).ok()?;
    // SAFETY: `address` está dentro da tela (x e y conferidos acima), mapeada no kernel desde o boot;
    // a leitura é de 32 bits alinhada, e só o autoteste a faz.
    Some(unsafe { core::ptr::read_volatile(core::ptr::with_exposed_provenance::<u32>(address)) })
}

/// A largura, a altura e o `stride` em bytes da tela, se há tela XRGB.
#[cfg(feature = "selftest")]
pub fn size() -> Option<(u32, u32, u32)> {
    let layout = (*LAYOUT.lock())?;
    Some((layout.width, layout.height, layout.pitch))
}
