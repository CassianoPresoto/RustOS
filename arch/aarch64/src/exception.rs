//! Tabela de vetores do EL1 e o despacho de exceções e IRQs para `hal::Handlers`.
//! O kernel roda em EL1h: a IRQ e a exceção vinda de EL0 salvam o quadro na pilha da própria
//! tarefa, onde ele sobrevive à troca de contexto. Falha síncrona e SError de EL1h passam para a
//! pilha de falhas desta CPU (o bloco por CPU, achado pelo `TPIDR_EL1`).

use core::arch::{asm, global_asm};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use rustos_hal::abi::{SyscallArgs, USER_TOP};
use rustos_hal::{Fault, FaultKind, Handlers, Irq, STACK_PAINT, VirtAddr};

use crate::percpu::prefix;
use crate::prefix::OFFSET_FAULT_STACK_TOP;
use crate::{esr, gic_cpu, usercopy};

/// Registradores salvos na entrada, na ordem que o `global_asm!` abaixo grava. Os dois últimos
/// só valem para exceção vinda de EL0: o `SP_EL0` e o `TPIDR_EL0` do usuário.
#[repr(C)]
struct Frame {
    x: [u64; 31],
    elr: u64,
    spsr: u64,
    sp_el0: u64,
    tpidr_el0: u64,
    _align: u64,
}

const FRAME_SIZE: usize = size_of::<Frame>();
const _: () = assert!(FRAME_SIZE == 288 && FRAME_SIZE.is_multiple_of(16));
const SP_EL0_AT: usize = core::mem::offset_of!(Frame, sp_el0);
const SPSR_AT: usize = core::mem::offset_of!(Frame, spsr);
const ELR_AT: usize = core::mem::offset_of!(Frame, elr);
const _: () = assert!(ELR_AT == 248 && SPSR_AT == 256 && SP_EL0_AT == 264);
/// `BRK` tem 4 bytes e o `ELR_EL1` aponta para ele: retornar exige somar isto.
const BRK_LEN: u64 = 4;
/// Bytes abaixo do quadro que a saída para EL0 repinta, sem usar pilha.
const SCRUB_WINDOW_BYTES: usize = 256;
/// As quatro metades de 16 bits do padrão de pintura, para o `movz`/`movk`.
const PAINT_0: u64 = STACK_PAINT & 0xFFFF;
const PAINT_1: u64 = STACK_PAINT >> 16 & 0xFFFF;
const PAINT_2: u64 = STACK_PAINT >> 32 & 0xFFFF;
const PAINT_3: u64 = STACK_PAINT >> 48;

static HANDLERS: AtomicPtr<Handlers> = AtomicPtr::new(ptr::null_mut());

// Cada entrada guarda x0, x1 e o índice (0–15): grupos EL1t, EL1h, EL0 A64, EL0 A32, cada um com
// síncrona, IRQ, FIQ e SError, 0x80 bytes por entrada. `vector_fault` usa a pilha de falhas do
// bloco por CPU (o `SP_EL0` precoce se ela não existe) e guarda o `x0` no `TPIDRRO_EL0`, que a
// saída para EL0 zera. Essa saída limpa a pilha do kernel e restaura `SP_EL0` e `TPIDR_EL0`.
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
    msr tpidrro_el0, x0
    mrs x0, tpidr_el1
    cbz x0, 8f
    ldr x0, [x0, #{fault_top}]
    cbz x0, 8f
    msr spsel, #0
    mov sp, x0
    b 9f
8:
    msr spsel, #0
9:
    mrs x0, tpidrro_el0
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
    stp x2, x3, [sp, #{elr_at}]
    tst x3, #0xf
    b.ne 1f
    mrs x4, sp_el0
    mrs x5, tpidr_el0
    stp x4, x5, [sp, #{sp_el0_at}]
1:
    mov x0, sp
    bl {dispatch}
    ldr x3, [sp, #{spsr_at}]
    tst x3, #0xf
    b.ne 2f
    adrp x0, {scrub_on}
    ldrb w0, [x0, :lo12:{scrub_on}]
    cbz w0, 4f
    mov x0, sp
    bl {leave_user}
    sub x0, sp, #{window}
    movz x1, #{paint_0}
    movk x1, #{paint_1}, lsl #16
    movk x1, #{paint_2}, lsl #32
    movk x1, #{paint_3}, lsl #48
    mov x2, #{window} / 16
3:
    stp x1, x1, [x0], #16
    subs x2, x2, #1
    b.ne 3b
4:
    ldp x4, x5, [sp, #{sp_el0_at}]
    msr sp_el0, x4
    msr tpidr_el0, x5
    msr tpidrro_el0, xzr
2:
    ldp x2, x3, [sp, #{elr_at}]
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
    fault_top = const OFFSET_FAULT_STACK_TOP,
    elr_at = const ELR_AT,
    spsr_at = const SPSR_AT,
    sp_el0_at = const SP_EL0_AT,
    window = const SCRUB_WINDOW_BYTES,
    paint_0 = const PAINT_0,
    paint_1 = const PAINT_1,
    paint_2 = const PAINT_2,
    paint_3 = const PAINT_3,
    dispatch = sym dispatch,
    leave_user = sym leave_user,
    scrub_on = sym SCRUB_ON,
);

/// A volta a EL0 chama `leave_user` e repinta a janela abaixo do quadro. Só o autoteste a desliga,
/// para medir o custo.
static SCRUB_ON: AtomicBool = AtomicBool::new(true);

/// Liga ou desliga a limpeza da pilha do kernel na volta a EL0.
pub fn set_scrub(on: bool) {
    SCRUB_ON.store(on, Ordering::Relaxed);
}

/// Vetores antes de haver memória: `handlers.fault` não volta. `install` os troca.
///
/// # Safety
/// Em EL1 com IRQ mascarada, na entrada, com 8 KiB livres abaixo da pilha viva.
pub unsafe fn install_early(handlers: &'static Handlers) {
    // A pilha que a exceção usa (`SP_EL1` em EL1t, `SP_EL0` em EL1h) vai para esses 8 KiB, e o
    // `TPIDR_EL1` zerado diz ao vetor que ainda não há bloco por CPU.
    HANDLERS.store(ptr::from_ref(handlers).cast_mut(), Ordering::Release);
    // SAFETY: `SP_EL1` só se escreve com `SPSel` em 1; sem acesso à pilha entre as trocas.
    // `isb` publica o `VBAR`.
    unsafe {
        asm!(
            "mrs {sel}, spsel",
            "mov {top}, sp",
            "sub {top}, {top}, #0x2000",
            "cbnz {sel}, 1f",
            "msr spsel, #1",
            "mov sp, {top}",
            "msr spsel, #0",
            "b 2f",
            "1:",
            "msr spsel, #0",
            "mov sp, {top}",
            "msr spsel, #1",
            "2:",
            "adrp {top}, rustos_vectors",
            "add {top}, {top}, :lo12:rustos_vectors",
            "msr vbar_el1, {top}",
            "msr tpidr_el1, xzr",
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
/// Em EL1h com interrupções mascaradas, depois de `percpu::install` nesta CPU. `stack_top` é o
/// fim, alinhado a 16 bytes, de uma pilha mapeada que só as falhas usam (fica no bloco por CPU).
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
/// Em EL1h, mascarada, depois de `install` e de `percpu::install` nesta CPU. `stack_top` é o
/// fim, alinhado a 16, de uma pilha mapeada só das falhas desta CPU.
pub unsafe fn install_secondary(stack_top: VirtAddr) -> Result<(), InstallError> {
    let selected: u64;
    // SAFETY: leitura de registrador de estado em EL1, sem efeito colateral.
    unsafe { asm!("mrs {}, spsel", out(reg) selected, options(nomem, nostack, preserves_flags)) };
    if selected & 1 == 0 {
        return Err(InstallError::NotOnEl1h);
    }
    prefix()
        .fault_stack_top
        .store(stack_top.as_u64(), Ordering::Release);
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
    let handlers = handlers();
    match index % 4 {
        1 => {
            let intid = gic_cpu::acknowledge();
            (handlers.irq)(Irq(intid));
            gic_cpu::end_of_interrupt(intid);
            (handlers.preempt)();
        }
        0 | 3 => synchronous(frame, index, handlers),
        _ => {
            let fault = Fault {
                kind: FaultKind::Other,
                pc: VirtAddr::new(frame.elr),
                code: index,
                from_user: false,
                user_sp: 0,
            };
            (handlers.fault)(&fault);
            park();
        }
    }
}

fn handlers() -> &'static Handlers {
    // SAFETY: `HANDLERS` só recebe um `&'static Handlers`, em `install`.
    match unsafe { HANDLERS.load(Ordering::Acquire).as_ref() } {
        Some(handlers) => handlers,
        None => park(),
    }
}

/// Grupo 8 do vetor: síncrona de EL0 em AArch64. A SError (índice 11) é fatal mesmo de EL0.
const EL0_SYNC: u64 = 8;

fn synchronous(frame: &mut Frame, index: u64, handlers: &Handlers) {
    let syndrome = read_esr();
    if index == EL0_SYNC && esr::is_svc64(syndrome) {
        let regs = &frame.x;
        let args = SyscallArgs {
            number: regs[8],
            args: [regs[0], regs[1], regs[2], regs[3], regs[4], regs[5]],
        };
        let ret = (handlers.syscall)(&args);
        frame.x[0] = ret.status.cast_unsigned();
        frame.x[1] = ret.value;
        return;
    }
    let far = read_far();
    let kind = esr::decode(syndrome, far);
    if index == 4
        && usercopy::is_usercopy_fault(frame.elr)
        && matches!(kind, FaultKind::PageFault { .. })
        && far < USER_TOP
    {
        frame.elr = usercopy::recovery_address();
        return;
    }
    let from_user = index == EL0_SYNC;
    let fault = Fault {
        kind,
        pc: VirtAddr::new(frame.elr),
        code: syndrome,
        from_user,
        user_sp: if from_user { frame.sp_el0 } else { 0 },
    };
    (handlers.fault)(&fault);
    if !kind.is_recoverable() {
        park();
    }
    if kind == FaultKind::Breakpoint {
        frame.elr += BRK_LEN;
    }
}

/// Última coisa antes do `eret` para EL0, com o quadro de kernel já morto abaixo de `frame`.
extern "C" fn leave_user(frame: usize) {
    (handlers().leave_user)(frame);
}
