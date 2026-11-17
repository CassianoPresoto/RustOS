//! Raiz de composição do RustOS: o único crate que conhece todos os adaptadores.
//! Aqui se decide qual implementação atende cada porta; política mora em `core/`.

#![no_std]
#![no_main]

extern crate alloc;

#[cfg(not(target_os = "none"))]
compile_error!("o kernel só compila para alvos bare-metal: use `cargo xtask build`");

mod boot;
#[macro_use]
mod console;
mod fmt;
mod frames;
mod heap;
mod interrupts;
mod memory;
mod panic;
mod platform;
mod random;
mod screen;
mod sync;
mod system;
mod vm;

use alloc::boxed::Box;

use rustos_boot_limine::LimineBoot;
use rustos_hal::{BootInfo, PAGE_SIZE};
use rustos_shell::DisplayInfo;

use crate::boot::BootRecord;

/// O `cargo xtask smoke` espera esta linha na serial; mudar aqui quebra o CI.
pub const BOOT_OK_MARKER: &str = "[rustos] boot ok";

/// Páginas da pilha do kernel: 64 KiB, o mínimo que o Limine garante e com que o
/// kernel já rodava, mais uma página de guarda abaixo.
const KERNEL_STACK_PAGES: u64 = 16;

/// O que passa da pilha do bootloader para a do kernel.
struct Handoff {
    boot: BootRecord,
    memory: memory::Memory,
    vm: vm::Vm,
    entry_cycles: u64,
}

#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    let entry_cycles = platform::cycles();
    // Até o console existir, pânico não aparece em lugar nenhum (DEBT-0005).
    // SAFETY: primeira coisa na entrada; nada reaproveitou a memória do bootloader.
    let limine = match unsafe { LimineBoot::from_bootloader() } {
        Ok(boot) => boot,
        Err(error) => panic!("boot protocol: {error:?}"),
    };
    // SAFETY: chamada única; as regiões utilizáveis estão livres e no mapa direto.
    let mut memory = match unsafe { memory::Memory::init(&limine) } {
        Ok(memory) => memory,
        Err(error) => panic!("memory: {error:?}"),
    };
    // Daqui em diante nada lê a resposta do Limine: a memória dela volta ao alocador.
    let boot = BootRecord::capture(&limine);
    platform::harden();
    // SAFETY: o mapa direto do Limine cobre todo frame do alocador e toda região de `boot`.
    let mut vm = match unsafe { vm::Vm::build(&boot, &mut memory.frames) } {
        Ok(vm) => vm,
        Err(error) => panic!("vm: {error:?}"),
    };
    if let Err(error) = memory::Memory::place_windows(&vm.occupied()) {
        panic!("address windows: {error:?}");
    }
    vm.activate();
    let top = match memory.map_stack(vm.space_mut(), KERNEL_STACK_PAGES) {
        Ok(top) => top,
        Err(error) => panic!("kernel stack: {error:?}"),
    };
    let handoff = Box::into_raw(Box::new(Handoff {
        boot,
        memory,
        vm,
        entry_cycles,
    }));
    // SAFETY: `top` é o fim de uma pilha recém-mapeada e só nossa; a continuação não
    // volta, e `handoff` vem de `Box::into_raw`.
    unsafe { platform::call_on_stack(top, run_on_kernel_stack, handoff.cast()) }
}

/// # Safety
/// `arg` é o ponteiro de `Box::into_raw` de um `Handoff`, entregue uma vez só.
unsafe extern "C" fn run_on_kernel_stack(arg: *mut ()) -> ! {
    // SAFETY: contrato da função.
    let handoff = unsafe { Box::from_raw(arg.cast::<Handoff>()) };
    boot_kernel(*handoff)
}

fn boot_kernel(handoff: Handoff) -> ! {
    let Handoff {
        mut boot,
        mut memory,
        vm,
        entry_cycles,
    } = handoff;
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

    let interrupts_on = interrupts::init(&boot, &mut memory);
    let reclaimed = memory.reclaim_bootloader(&mut boot);
    let source = random::init(&boot_marks(&boot, &memory, entry_cycles));
    report(&boot, &memory, display, reclaimed);
    report_vm(&vm, boot.direct_map_offset());
    report_random(source);
    // SAFETY: o mapa direto cobre todo frame utilizável, de onde vem o frame do teste.
    let frames_ok = unsafe { memory.self_test(boot.direct_map_offset()) };
    kprintln!(
        "[rustos] frame self-test: {}",
        if frames_ok { "ok" } else { "FAILED" }
    );
    let heap_ok = heap::self_test();
    kprintln!(
        "[rustos] heap self-test: {}",
        if heap_ok { "ok" } else { "FAILED" }
    );
    let wx_ok = report_wx(&vm);
    let irq_ok = interrupts_on && system::interrupt_self_test();
    if !frames_ok || !heap_ok || !wx_ok || !irq_ok {
        rustos_hal::Cpu::halt_forever(&platform::cpu());
    }
    kprintln!("{BOOT_OK_MARKER}");
    let mut system = system::System::new(&boot, &memory, display);
    system::run_shell(&mut system)
}

/// O que o boot tem de impredizível: contadores de ciclo e os tempos do bootloader.
fn boot_marks(boot: &impl BootInfo, memory: &memory::Memory, entry_cycles: u64) -> [u64; 7] {
    let timeline = boot.timeline();
    let field = |read: fn(&rustos_hal::BootTimeline) -> u64| timeline.as_ref().map_or(0, read);
    [
        entry_cycles,
        platform::cycles(),
        field(|t| t.reset_us),
        field(|t| t.loader_start_us),
        field(|t| t.kernel_handoff_us),
        platform::now_ns().unwrap_or(0),
        memory.frames.free_frames() as u64,
    ]
}

fn report_vm(vm: &vm::Vm, direct_map_offset: u64) {
    let stats = vm.stats();
    kprintln!(
        "[rustos] vm: direct map at {direct_map_offset:#x} in {} 2M and {} 4K pages, {} KiB of tables, stack {} KiB",
        stats.large_pages,
        stats.small_pages,
        (stats.table_frames * PAGE_SIZE) >> 10,
        (KERNEL_STACK_PAGES * PAGE_SIZE) >> 10
    );
}

/// Percorre as tabelas novas: nenhuma página gravável e executável (guidelines 09).
fn report_wx(vm: &vm::Vm) -> bool {
    let verdict = vm.check_wx();
    match verdict {
        Ok(pages) => kprintln!("[rustos] w^x: ok ({pages} pages)"),
        Err(violation) => {
            kprintln!("[rustos] w^x: violation {violation:?}");
            kprintln!("[rustos] w^x: FAILED");
        }
    }
    verdict.is_ok()
}

fn report_random(source: random::Source) {
    match source {
        random::Source::Hardware(name) => kprintln!("[rustos] random: {name}"),
        random::Source::Jitter => kprintln!("[rustos] random: jitter (weak)"),
    }
    if let Some(bench) = random::benchmark() {
        kprintln!(
            "[rustos] random bench: chacha20 {} MB/s, reseed {} us",
            bench.mb_per_s,
            bench.reseed_ns / 1000
        );
    }
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

fn report(
    boot: &impl BootInfo,
    memory: &memory::Memory,
    display: Option<DisplayInfo>,
    reclaimed: u64,
) {
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
    if let Some(image) = boot.kernel_image() {
        kprintln!(
            "[rustos] kernel base: {:#x} (physical {:#x})",
            image.virtual_base.as_u64(),
            image.physical_base.as_u64()
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
        "[rustos] reclaimed: {} KiB from the bootloader",
        reclaimed >> 10
    );
    kprintln!(
        "[rustos] frames: {} free of {} tracked, metadata {} bytes",
        memory.frames.free_frames(),
        memory.frames.total_frames(),
        memory.metadata_bytes
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
    let mut line = fmt::Buffer::<128>::new();
    for protection in platform::protections() {
        let mark = if protection.enabled { '+' } else { '-' };
        core::fmt::Write::write_fmt(&mut line, format_args!(" {mark}{}", protection.name)).ok();
    }
    if !line.as_str().is_empty() {
        kprintln!("[rustos] protections:{}", line.as_str());
    }
}
