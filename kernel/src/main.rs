//! Raiz de composição do RustOS: o único crate que conhece todos os adaptadores.
//! Aqui se decide qual implementação atende cada porta; política mora em `core/`.

#![no_std]
#![no_main]

#[cfg(not(target_os = "none"))]
compile_error!("o kernel só compila para alvos bare-metal: use `cargo xtask build`");

#[macro_use]
mod console;
mod memory;
mod panic;
mod platform;

use rustos_boot_limine::LimineBoot;
use rustos_hal::{BootInfo, Cpu};

/// O `cargo xtask smoke` espera esta linha na serial; mudar aqui quebra o CI.
pub const BOOT_OK_MARKER: &str = "[rustos] boot ok";

#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    // Até o console existir, pânico não aparece em lugar nenhum (DEBT-0005).
    // SAFETY: primeira coisa na entrada; nada reaproveitou a memória do bootloader.
    let boot = match unsafe { LimineBoot::from_bootloader() } {
        Ok(boot) => boot,
        Err(error) => panic!("boot protocol: {error:?}"),
    };
    // SAFETY: chamada única; as regiões utilizáveis estão livres e no mapa direto.
    let mut memory = match unsafe { memory::Memory::init(&boot) } {
        Ok(memory) => memory,
        Err(error) => panic!("memory: {error:?}"),
    };
    match platform::console(&boot, &mut memory.frames) {
        Ok(console) => console::install(console),
        Err(error) => panic!("console: {error:?}"),
    }

    report(&boot, &memory);
    // SAFETY: o mapa direto cobre todo frame utilizável, de onde vem o frame do teste.
    let frames_ok = unsafe { memory.self_test(boot.direct_map_offset()) };
    kprintln!(
        "[rustos] frame self-test: {}",
        if frames_ok { "ok" } else { "FAILED" }
    );
    if frames_ok {
        kprintln!("{BOOT_OK_MARKER}");
    }
    platform::cpu().halt_forever()
}

fn report(boot: &impl BootInfo, memory: &memory::Memory) {
    kprintln!(
        "[rustos] RustOS {} ({})",
        env!("CARGO_PKG_VERSION"),
        platform::ARCH
    );
    if let Some(loader) = boot.bootloader() {
        kprintln!("[rustos] bootloader: {} {}", loader.name, loader.version);
    }
    if let Some(timeline) = boot.timeline() {
        kprintln!(
            "[rustos] boot timeline: firmware {} us, bootloader {} us",
            timeline.firmware_us(),
            timeline.loader_us()
        );
    }
    let summary = memory.summary;
    kprintln!(
        "[rustos] memory: {} KiB usable, {} KiB other, {} regions",
        summary.usable_bytes >> 10,
        summary.other_bytes >> 10,
        summary.regions
    );
    kprintln!(
        "[rustos] frames: {} free of {} tracked, bitmap {} bytes",
        memory.frames.free_frames(),
        memory.frames.total_frames(),
        memory.bitmap_bytes
    );
}
