//! Raiz de composição do RustOS: o único crate que conhece todos os adaptadores.
//! Aqui se decide qual implementação atende cada porta; política mora em `core/`.

#![no_std]
#![no_main]

#[cfg(not(target_os = "none"))]
compile_error!("o kernel só compila para alvos bare-metal: use `cargo xtask build`");

#[macro_use]
mod console;
mod fmt;
mod memory;
mod panic;
mod platform;
mod screen;
mod system;

use rustos_boot_limine::LimineBoot;
use rustos_hal::BootInfo;
use rustos_shell::DisplayInfo;

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

    let display = boot.framebuffer().and_then(|info| {
        // SAFETY: chamada única; o framebuffer veio do bootloader e só a tela o toca.
        match unsafe { screen::Screen::new(&info, &mut memory) } {
            Ok(screen) => {
                let display = display_info(&screen);
                console::install_screen(screen);
                Some(display)
            }
            Err(error) => {
                kprintln!("[rustos] screen: {error:?}");
                None
            }
        }
    });

    report(&boot, &memory, display);
    // SAFETY: o mapa direto cobre todo frame utilizável, de onde vem o frame do teste.
    let frames_ok = unsafe { memory.self_test(boot.direct_map_offset()) };
    kprintln!(
        "[rustos] frame self-test: {}",
        if frames_ok { "ok" } else { "FAILED" }
    );
    if !frames_ok {
        rustos_hal::Cpu::halt_forever(&platform::cpu());
    }
    kprintln!("{BOOT_OK_MARKER}");
    system::run_shell(&mut system::System::new(&boot, &memory, display))
}

fn display_info(screen: &screen::Screen) -> DisplayInfo {
    let (width, height) = screen.size();
    let (cols, rows) = screen.grid();
    let narrow = |value: usize| u16::try_from(value).unwrap_or(u16::MAX);
    DisplayInfo {
        width: u32::try_from(width).unwrap_or(u32::MAX),
        height: u32::try_from(height).unwrap_or(u32::MAX),
        cols: narrow(cols),
        rows: narrow(rows),
    }
}

fn report(boot: &impl BootInfo, memory: &memory::Memory, display: Option<DisplayInfo>) {
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
    match display {
        Some(display) => kprintln!(
            "[rustos] screen: {}x{}, grid {}x{}",
            display.width,
            display.height,
            display.cols,
            display.rows
        ),
        None => kprintln!("[rustos] screen: none"),
    }
}
