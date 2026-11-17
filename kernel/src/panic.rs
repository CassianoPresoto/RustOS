//! Pânico do kernel: escreve o que puder na serial e para a CPU.

use core::panic::PanicInfo;

use rustos_hal::Cpu;

#[panic_handler]
fn panic(info: &PanicInfo<'_>) -> ! {
    crate::console::print_panic(format_args!("[rustos] panic: {info}\n"));
    crate::platform::cpu().halt_forever()
}
