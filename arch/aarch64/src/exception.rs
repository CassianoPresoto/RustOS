//! Tabela de vetores do EL1 e o despacho de exceções e IRQs para `hal::Handlers`.
//! O kernel roda em EL1h: a IRQ salva o quadro na pilha da própria tarefa, onde ele
//! sobrevive à troca de contexto. Falha síncrona e SError passam para a pilha de falhas.

use core::arch::{asm, global_asm};
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use rustos_hal::{Fault, FaultKind, Handlers, Irq, VirtAddr};

use crate::esr;
use crate::gic_cpu;

/// Registradores salvos na entrada, na ordem que o `global_asm!` abaixo grava.
#[repr(C)]
struct Frame {
    x: [u64; 31],
    elr: u64,
    spsr: u64,
    _align: u64,
}

const FRAME_SIZE: usize = size_of::<Frame>();
const _: () = assert!(FRAME_SIZE == 272 && FRAME_SIZE.is_multiple_of(16));
/// `BRK` tem 4 bytes e o `ELR_EL1` aponta para ele: retornar exige somar isto.
const BRK_LEN: u64 = 4;

static HANDLERS: AtomicPtr<Handlers> = AtomicPtr::new(ptr::null_mut());

// Cada entrada guarda x0, x1 e o índice (0–15) e salta para o corpo comum. As
// quatro entradas de um grupo: síncrona, IRQ, FIQ, SError; os grupos: EL1 com
// SP0, EL1 com SPx, EL0 AArch64, EL0 AArch32. Cada entrada ocupa 0x80 bytes.
// `vector_fault` (síncrona e SError de EL1h) empilha em `SP_EL0`, a pilha de falhas.
global_asm!(
    r#"
    .macro vector_entry index
    .balign 0x80
    sub sp, sp, #{frame}
    stp x0, x1, [sp]
    mov x1, #\index
    b rustos_exception_common
    .endm

    .macro vector_fault index
    .balign 0x80
    msr spsel, #0
    sub sp, sp, #{frame}
    stp x0, x1, [sp]
    mov x1, #\index
    b rustos_exception_common
    .endm

    .pushsection .text.rustos_vectors, "ax"
    .balign 0x800
    .global rustos_vectors
rustos_vectors:
    vector_entry 0
    vector_entry 1
    vector_entry 2
    vector_entry 3
    vector_fault 4
    vector_entry 5
    vector_entry 6
    vector_fault 7
    vector_entry 8
    vector_entry 9
    vector_entry 10
    vector_entry 11
    vector_entry 12
    vector_entry 13
    vector_entry 14
    vector_entry 15

    .balign 0x80
rustos_exception_common:
    stp x2, x3, [sp, #16]
    stp x4, x5, [sp, #32]
    stp x6, x7, [sp, #48]
    stp x8, x9, [sp, #64]
    stp x10, x11, [sp, #80]
    stp x12, x13, [sp, #96]
    stp x14, x15, [sp, #112]
    stp x16, x17, [sp, #128]
    stp x18, x19, [sp, #144]
    stp x20, x21, [sp, #160]
    stp x22, x23, [sp, #176]
    stp x24, x25, [sp, #192]
    stp x26, x27, [sp, #208]
    stp x28, x29, [sp, #224]
    str x30, [sp, #240]
    mrs x2, elr_el1
    mrs x3, spsr_el1
    stp x2, x3, [sp, #248]
    mov x0, sp
    bl {dispatch}
    ldp x2, x3, [sp, #248]
    msr elr_el1, x2
    msr spsr_el1, x3
    ldp x2, x3, [sp, #16]
    ldp x4, x5, [sp, #32]
    ldp x6, x7, [sp, #48]
    ldp x8, x9, [sp, #64]
    ldp x10, x11, [sp, #80]
    ldp x12, x13, [sp, #96]
    ldp x14, x15, [sp, #112]
    ldp x16, x17, [sp, #128]
    ldp x18, x19, [sp, #144]
    ldp x20, x21, [sp, #160]
    ldp x22, x23, [sp, #176]
    ldp x24, x25, [sp, #192]
    ldp x26, x27, [sp, #208]
    ldp x28, x29, [sp, #224]
    ldr x30, [sp, #240]
    ldp x0, x1, [sp]
    add sp, sp, #{frame}
    eret
    .popsection
    "#,
    frame = const FRAME_SIZE,
    dispatch = sym dispatch,
);

/// Vetores antes de haver memória: `handlers.fault` não volta. `install` os troca.
///
/// # Safety
/// Em EL1 com IRQ mascarada, na entrada, com 8 KiB livres abaixo da pilha (`SP_EL1` vai ali).
pub unsafe fn install_early(handlers: &'static Handlers) {
    HANDLERS.store(ptr::from_ref(handlers).cast_mut(), Ordering::Release);
    // SAFETY: `SP_EL1` só se escreve com `SPSel` em 1; sem acesso à pilha entre as trocas.
    // `isb` publica o `VBAR`.
    unsafe {
        asm!(
            "mrs {sel}, spsel",
            "cbnz {sel}, 1f",
            "mov {top}, sp",
            "sub {top}, {top}, #0x2000",
            "msr spsel, #1",
            "mov sp, {top}",
            "msr spsel, #0",
            "1:",
            "adrp {top}, rustos_vectors",
            "add {top}, {top}, :lo12:rustos_vectors",
            "msr vbar_el1, {top}",
            "isb",
            sel = out(reg) _,
            top = out(reg) _,
            options(nostack, preserves_flags),
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallError {
    /// O kernel ainda roda em `SP_EL0` (EL1t): `call_on_stack` passa para EL1h.
    NotOnEl1h,
}

/// Instala os vetores e a pilha de falhas e registra os handlers.
/// # Safety
/// Em EL1h com interrupções mascaradas. `stack_top` é o fim, alinhado a 16
/// bytes, de uma pilha mapeada que só as falhas usam (fica em `SP_EL0`).
pub unsafe fn install(
    handlers: &'static Handlers,
    stack_top: VirtAddr,
) -> Result<(), InstallError> {
    let selected: u64;
    // SAFETY: leitura de registrador de estado em EL1, sem efeito colateral.
    unsafe { asm!("mrs {}, spsel", out(reg) selected, options(nomem, nostack, preserves_flags)) };
    if selected & 1 == 0 {
        return Err(InstallError::NotOnEl1h);
    }
    HANDLERS.store(ptr::from_ref(handlers).cast_mut(), Ordering::Release);
    // SAFETY: o contrato desta função vale para a CPU de boot.
    unsafe { install_secondary(stack_top) }
}

/// Instala os vetores e a pilha de falhas desta CPU; os handlers já são os de `install`.
/// # Safety
/// Em EL1h com interrupções mascaradas, depois de `install` na CPU de boot. `stack_top` é
/// o fim, alinhado a 16 bytes, de uma pilha mapeada que só as falhas desta CPU usam.
pub unsafe fn install_secondary(stack_top: VirtAddr) -> Result<(), InstallError> {
    let selected: u64;
    // SAFETY: leitura de registrador de estado em EL1, sem efeito colateral.
    unsafe { asm!("mrs {}, spsel", out(reg) selected, options(nomem, nostack, preserves_flags)) };
    if selected & 1 == 0 {
        return Err(InstallError::NotOnEl1h);
    }
    // SAFETY: `SP_EL0` só se escreve com `SPSel` em 0; sem acesso à pilha entre as
    // trocas, e com `SPSel` voltando a 1 o `sp` do kernel é o de antes.
    unsafe {
        asm!(
            "msr spsel, #0",
            "mov sp, {top}",
            "msr spsel, #1",
            top = in(reg) stack_top.as_u64(),
            options(nomem, nostack, preserves_flags),
        );
    }
    // SAFETY: `rustos_vectors` é a tabela acima, alinhada a 2 KiB; `isb` publica o `VBAR`.
    unsafe {
        asm!(
            "adrp {t}, rustos_vectors",
            "add {t}, {t}, :lo12:rustos_vectors",
            "msr vbar_el1, {t}",
            "isb",
            t = out(reg) _,
            options(nostack, preserves_flags),
        );
    }
    Ok(())
}

/// Executa `brk #0`: a exceção volta para a instrução seguinte.
pub fn breakpoint() {
    // SAFETY: `brk` só gera a exceção síncrona que o handler instalado trata.
    unsafe { asm!("brk #0", options(nomem, nostack)) };
}

/// Lê um byte de `address`; sem mapeamento, gera o page fault.
pub fn probe_read(address: VirtAddr) {
    // SAFETY: o acesso só lê um byte; sem mapeamento vira a exceção que se quer provocar.
    unsafe {
        asm!("ldrb {v:w}, [{a}]", a = in(reg) address.as_u64(), v = out(reg) _, options(nostack, preserves_flags));
    }
}

fn read_esr() -> u64 {
    let esr: u64;
    // SAFETY: leitura de registrador de sistema em EL1, sem efeito colateral.
    unsafe { asm!("mrs {}, esr_el1", out(reg) esr, options(nomem, nostack, preserves_flags)) };
    esr
}

fn read_far() -> u64 {
    let far: u64;
    // SAFETY: leitura de registrador de sistema em EL1, sem efeito colateral.
    unsafe { asm!("mrs {}, far_el1", out(reg) far, options(nomem, nostack, preserves_flags)) };
    far
}

/// Para a CPU quando não há para onde voltar: o `eret` repetiria a falha.
fn park() -> ! {
    loop {
        // SAFETY: `wfe` só suspende o núcleo.
        unsafe { asm!("wfe", options(nomem, nostack, preserves_flags)) };
    }
}

/// Chamado pelo `global_asm!` com o quadro salvo e o índice da entrada (0–15).
extern "C" fn dispatch(frame: &mut Frame, index: u64) {
    // SAFETY: `HANDLERS` só recebe um `&'static Handlers`, em `install`.
    let Some(handlers) = (unsafe { HANDLERS.load(Ordering::Acquire).as_ref() }) else {
        park()
    };
    match index % 4 {
        1 => {
            let intid = gic_cpu::acknowledge();
            (handlers.irq)(Irq(intid));
            gic_cpu::end_of_interrupt(intid);
            (handlers.preempt)();
        }
        0 | 3 => {
            let syndrome = read_esr();
            let kind = esr::decode(syndrome, read_far());
            let fault = Fault {
                kind,
                pc: VirtAddr::new(frame.elr),
                code: syndrome,
            };
            (handlers.fault)(&fault);
            if !kind.is_recoverable() {
                park();
            }
            if kind == FaultKind::Breakpoint {
                frame.elr += BRK_LEN;
            }
        }
        _ => {
            let fault = Fault {
                kind: FaultKind::Other,
                pc: VirtAddr::new(frame.elr),
                code: index,
            };
            (handlers.fault)(&fault);
            park();
        }
    }
}
