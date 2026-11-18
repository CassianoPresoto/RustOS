//! GDT, TSS e IDT próprias, os stubs de entrada e o despacho para os `Handlers`.

use core::arch::{asm, global_asm};
use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

use rustos_hal::abi::{SyscallArgs, USER_TOP};
use rustos_hal::{Fault, FaultKind, Handlers, Irq, STACK_PAINT, VirtAddr};

use crate::fault::{
    self, DEBUG, DOUBLE_FAULT, ERROR_CODE_VECTORS, FIRST_IRQ_VECTOR, MACHINE_CHECK, NMI, PAGE_FAULT,
};
use crate::lapic::VECTOR_SPURIOUS;
use crate::percpu::prefix;
use crate::prefix::OFFSET_KERNEL_RSP;
use crate::segments::{
    GDT_LEN, GDT_LIMIT, IDT_LEN, IDT_LIMIT, IdtEntry, KERNEL_CODE, KERNEL_DATA, TSS_SELECTOR,
    TablePointer, Tss, USER_CODE, USER_DATA, build_gdt,
};
use crate::sysreg::{MSR_GS_BASE, rdmsr, wrmsr};
use crate::sysret::{RF, TF};
use crate::{apic, user, usercopy};

/// Topos das pilhas de #DF, NMI, #MC e #DB (IST1 a IST4), com página de guarda abaixo.
#[derive(Clone, Copy, Debug)]
pub struct IstStacks {
    pub double_fault: VirtAddr,
    pub nmi: VirtAddr,
    pub machine_check: VirtAddr,
    pub debug: VirtAddr,
}

/// O que vai em `TrapFrame::vector` numa entrada por `SYSCALL` (nenhum vetor da IDT chega a isso).
pub const SYSCALL_VECTOR: u64 = 0x100;
/// Bytes abaixo do quadro que a saída para o usuário repinta, sem usar pilha.
const SCRUB_WINDOW_BYTES: usize = 256;

/// Registradores salvos pela entrada comum, na ordem em que ficam na pilha.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct TrapFrame {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    r11: u64,
    r10: u64,
    r9: u64,
    r8: u64,
    rdi: u64,
    rsi: u64,
    rbp: u64,
    rbx: u64,
    rdx: u64,
    rcx: u64,
    rax: u64,
    vector: u64,
    error: u64,
    rip: u64,
    cs: u64,
    rflags: u64,
    rsp: u64,
    ss: u64,
}

/// Estado escrito uma vez por `install` e lido pela CPU; `Sync` porque só `install`
/// o escreve, antes de carregar a tabela.
struct Table<T>(UnsafeCell<T>);

// SAFETY: a escrita é única, em `install`, com interrupções mascaradas.
unsafe impl<T> Sync for Table<T> {}

/// GDT e TSS de uma CPU. Cada CPU tem as suas: `ltr` marca a TSS como ocupada na GDT, e
/// as pilhas IST são por CPU. A IDT é uma só, de todas.
#[derive(Debug)]
#[repr(C, align(16))]
pub struct CpuTables {
    gdt: [u64; GDT_LEN],
    tss: Tss,
}

impl CpuTables {
    pub const fn new() -> Self {
        Self {
            gdt: [0; GDT_LEN],
            tss: Tss::new(),
        }
    }
}

impl Default for CpuTables {
    fn default() -> Self {
        Self::new()
    }
}

static IDT: Table<[IdtEntry; IDT_LEN]> = Table(UnsafeCell::new([IdtEntry::MISSING; IDT_LEN]));
static HANDLERS: AtomicPtr<Handlers> = AtomicPtr::new(ptr::null_mut());

// Cada stub ocupa 16 bytes: o gate do vetor `v` aponta para `stubs + 16 * v`. `CS` em anel 3 pede
// `swapgs` na entrada e na saída; NMI, `#DF`, `#MC` e `#DB` decidem pelo sinal de `IA32_GS_BASE`.
// A saída para o usuário limpa a pilha do kernel e exige `RIP` e `RSP` canônicos, porque um
// `iretq` que falhasse já teria o `GS` do usuário.
global_asm!(
    r#"
    .section .text.rustos_isr, "ax"
    .balign 16
    .global rustos_isr_stubs
rustos_isr_stubs:
    .set vec, 0
    .rept 256
    .balign 16
    .if vec == 1
    jmp rustos_debug_entry
    .elseif vec == 2
    push 0
    push 2
    jmp rustos_isr_paranoid
    .elseif vec == 8
    push 8
    jmp rustos_isr_paranoid
    .elseif vec == 18
    push 0
    push 18
    jmp rustos_isr_paranoid
    .else
    .if vec < 32 && (({ERROR_CODE_VECTORS} >> vec) & 1) != 0
    .else
    push 0
    .endif
    push vec
    jmp rustos_isr_common
    .endif
    .set vec, vec + 1
    .endr

    .balign 16
rustos_isr_common:
    test byte ptr [rsp + 24], 3
    jz 1f
    swapgs
1:
    .global rustos_isr_common_swapped
rustos_isr_common_swapped:
    push rax
    push rcx
    push rdx
    push rbx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    cld
    mov rdi, rsp
    call {dispatch}
    test byte ptr [rsp + {CS_AT}], 3
    jz 3f
    cmp byte ptr [rip + {SCRUB_ON}], 0
    je 2f
    mov rdi, rsp
    call {leave_user}
    lea rdi, [rsp - {WINDOW}]
    mov ecx, {WINDOW} / 8
    mov rax, {PAINT}
    rep stosq
2:
    mov rax, [rsp + {RSP_AT}]
    mov rcx, rax
    shl rcx, 16
    sar rcx, 16
    cmp rcx, rax
    jne 4f
    mov rax, [rsp + {RIP_AT}]
    mov rcx, rax
    shl rcx, 16
    sar rcx, 16
    cmp rcx, rax
    jne 4f
    cmp qword ptr [rsp + {VECTOR_AT}], {SYSCALL_VECTOR}
    jne 3f
    cmp byte ptr [rip + {SYSRET_ON}], 0
    je 3f
    mov rax, [rsp + {RIP_AT}]
    cmp rax, [rsp + {RCX_AT}]
    jne 3f
    mov rcx, [rsp + {RFLAGS_AT}]
    cmp rcx, [rsp + {R11_AT}]
    jne 3f
    test rcx, {RF_TF}
    jnz 3f
    cmp qword ptr [rsp + {CS_AT}], {USER_CS}
    jne 3f
    cmp qword ptr [rsp + {SS_AT}], {USER_SS}
    jne 3f
    mov rcx, {USER_TOP}
    cmp rax, rcx
    jae 3f
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    pop rdx
    pop rcx
    pop rax
    mov rsp, [rsp + 40]
    swapgs
    sysretq
3:
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    pop rdx
    pop rcx
    pop rax
    add rsp, 16
    test byte ptr [rsp + 8], 3
    jz 5f
    swapgs
5:
    iretq
4:
    mov rdi, rsp
    call {bad_return}
    ud2

    .balign 16
rustos_isr_paranoid:
    push rax
    push rcx
    push rdx
    push rbx
    push rbp
    push rsi
    push rdi
    push r8
    push r9
    push r10
    push r11
    push r12
    push r13
    push r14
    push r15
    mov ecx, {GS_BASE}
    rdmsr
    xor ebx, ebx
    test edx, edx
    js 6f
    swapgs
    mov ebx, 1
6:
    cld
    mov rdi, rsp
    call {dispatch}
    test ebx, ebx
    jz 7f
    swapgs
7:
    pop r15
    pop r14
    pop r13
    pop r12
    pop r11
    pop r10
    pop r9
    pop r8
    pop rdi
    pop rsi
    pop rbp
    pop rbx
    pop rdx
    pop rcx
    pop rax
    add rsp, 16
    iretq

    .balign 16
rustos_debug_entry:
    test byte ptr [rsp + 8], 3
    jnz 8f
    push 0
    push 1
    jmp rustos_isr_paranoid
8:
    swapgs
    push rax
    push rcx
    mov rcx, gs:[{KERNEL_RSP}]
    mov rax, [rsp + 48]
    mov [rcx - 8], rax
    mov rax, [rsp + 40]
    mov [rcx - 16], rax
    mov rax, [rsp + 32]
    mov [rcx - 24], rax
    mov rax, [rsp + 24]
    mov [rcx - 32], rax
    mov rax, [rsp + 16]
    mov [rcx - 40], rax
    mov rax, rsp
    lea rsp, [rcx - 40]
    push 0
    push 1
    mov rcx, [rax]
    mov rax, [rax + 8]
    jmp rustos_isr_common_swapped
    "#,
    ERROR_CODE_VECTORS = const ERROR_CODE_VECTORS,
    dispatch = sym trap_dispatch,
    leave_user = sym leave_user,
    bad_return = sym bad_user_return,
    CS_AT = const FRAME_CS_OFFSET,
    RSP_AT = const FRAME_RSP_OFFSET,
    RIP_AT = const FRAME_RIP_OFFSET,
    WINDOW = const SCRUB_WINDOW_BYTES,
    PAINT = const STACK_PAINT,
    GS_BASE = const MSR_GS_BASE,
    KERNEL_RSP = const OFFSET_KERNEL_RSP,
    VECTOR_AT = const core::mem::offset_of!(TrapFrame, vector),
    RCX_AT = const core::mem::offset_of!(TrapFrame, rcx),
    R11_AT = const core::mem::offset_of!(TrapFrame, r11),
    RFLAGS_AT = const core::mem::offset_of!(TrapFrame, rflags),
    SS_AT = const core::mem::offset_of!(TrapFrame, ss),
    SYSCALL_VECTOR = const SYSCALL_VECTOR,
    SYSRET_ON = sym SYSRET_ON,
    SCRUB_ON = sym SCRUB_ON,
    RF_TF = const RF | TF,
    USER_CS = const USER_CODE,
    USER_SS = const USER_DATA,
    USER_TOP = const USER_TOP,
);

/// A saída de syscall pode usar `SYSRET` quando a conferência de `sysret::can_sysret` passa; sem
/// isto, sempre `IRETQ`. O autoteste liga e desliga para medir.
static SYSRET_ON: AtomicBool = AtomicBool::new(true);

/// A volta ao usuário chama `leave_user` e repinta a janela abaixo do quadro. Só o autoteste a
/// desliga, para medir o custo.
static SCRUB_ON: AtomicBool = AtomicBool::new(true);

/// Liga ou desliga a limpeza da pilha do kernel na volta ao usuário.
pub fn set_scrub(on: bool) {
    SCRUB_ON.store(on, Ordering::Relaxed);
}

/// Liga ou desliga a saída por `SYSRET`.
pub fn set_sysret(on: bool) {
    SYSRET_ON.store(on, Ordering::Relaxed);
}

const FRAME_RIP_OFFSET: usize = core::mem::offset_of!(TrapFrame, rip);
const FRAME_CS_OFFSET: usize = core::mem::offset_of!(TrapFrame, cs);
const FRAME_RSP_OFFSET: usize = core::mem::offset_of!(TrapFrame, rsp);

unsafe extern "C" {
    static rustos_isr_stubs: u8;
}

fn handlers() -> &'static Handlers {
    // SAFETY: `install` guardou um `&'static Handlers` antes de carregar a IDT.
    match unsafe { HANDLERS.load(Ordering::Acquire).as_ref() } {
        Some(handlers) => handlers,
        None => halt(),
    }
}

extern "C" fn trap_dispatch(frame: &mut TrapFrame) {
    let handlers = handlers();
    if frame.vector == SYSCALL_VECTOR {
        let args = SyscallArgs {
            number: frame.rax,
            args: [
                frame.rdi, frame.rsi, frame.rdx, frame.r10, frame.r8, frame.r9,
            ],
        };
        let ret = (handlers.syscall)(&args);
        frame.rax = ret.status.cast_unsigned();
        frame.rdx = ret.value;
        return;
    }
    let vector = u8::try_from(frame.vector).unwrap_or(u8::MAX);
    if vector < FIRST_IRQ_VECTOR {
        let from_user = frame.cs & 3 == 3;
        if vector == PAGE_FAULT && !from_user && usercopy::is_usercopy_fault(frame.rip) {
            frame.rip = usercopy::recovery_address();
            return;
        }
        let cr2 = if vector == PAGE_FAULT { read_cr2() } else { 0 };
        let mut report = fault::decode(vector, frame.error, frame.rip, cr2);
        report.from_user = from_user && fault::user_fault_kills_only_the_process(vector);
        report.user_sp = if report.from_user { frame.rsp } else { 0 };
        (handlers.fault)(&report);
        if !report.kind.is_recoverable() {
            halt();
        }
        return;
    }
    (handlers.irq)(Irq(u32::from(vector)));
    if vector != VECTOR_SPURIOUS {
        apic::eoi();
    }
    (handlers.preempt)();
}

/// Última coisa antes do `iretq` para o usuário, com o quadro de kernel já morto abaixo de `frame`.
extern "C" fn leave_user(frame: usize) {
    (handlers().leave_user)(frame);
}

/// O quadro de volta ao usuário tem `RIP` ou `RSP` não canônico: o `iretq` falharia já com o
/// `GS` do usuário. O processo morre como por um `#GP`.
extern "C" fn bad_user_return(frame: &mut TrapFrame) -> ! {
    let report = Fault {
        kind: FaultKind::GeneralProtection,
        pc: VirtAddr::new(frame.rip),
        code: fault::raw_code(fault::GENERAL_PROTECTION, 0),
        from_user: true,
        user_sp: frame.rsp,
    };
    (handlers().fault)(&report);
    halt()
}

fn halt() -> ! {
    loop {
        // SAFETY: ring 0; `cli` e `hlt` só param esta CPU.
        unsafe { asm!("cli", "hlt", options(nomem, nostack)) };
        spin_loop();
    }
}

fn read_cr2() -> u64 {
    let value: u64;
    // SAFETY: ler `CR2` não tem efeito; o kernel roda em ring 0.
    unsafe { asm!("mov {}, cr2", out(reg) value, options(nomem, nostack, preserves_flags)) };
    value
}

/// IST de cada vetor que precisa de pilha própria: #DF, NMI, #MC e #DB.
const fn ist_for(vector: u8) -> u8 {
    match vector {
        DOUBLE_FAULT => 1,
        NMI => 2,
        MACHINE_CHECK => 3,
        DEBUG => 4,
        _ => 0,
    }
}

/// Monta a IDT de todas as CPUs, carrega as tabelas desta e despacha para `handlers`.
///
/// # Safety
/// Ring 0, uma vez, com IF zerado, antes de liberar outra CPU. Contrato de `load_cpu`.
pub unsafe fn install(ist: IstStacks, handlers: &'static Handlers, tables: &'static mut CpuTables) {
    HANDLERS.store(ptr::from_ref(handlers).cast_mut(), Ordering::Release);
    let stubs = ptr::addr_of!(rustos_isr_stubs).addr() as u64;
    let mut idt = [IdtEntry::MISSING; IDT_LEN];
    for (vector, entry) in (0..=u8::MAX).zip(idt.iter_mut()) {
        *entry = IdtEntry::interrupt(stubs + 16 * u64::from(vector), ist_for(vector));
    }
    // SAFETY: única escrita, antes de qualquer `lidt` (contrato de `install`).
    unsafe { IDT.0.get().write(idt) };
    // SAFETY: o contrato de `install` vale para esta CPU.
    unsafe { load_cpu(ist, tables) };
}

/// IDT mínima sobre a GDT do bootloader, sem IST nem APIC; `install` a substitui.
///
/// # Safety
/// Ring 0 com IF zerado, na entrada do kernel, com a GDT do bootloader carregada.
pub unsafe fn install_early(handlers: &'static Handlers) {
    HANDLERS.store(ptr::from_ref(handlers).cast_mut(), Ordering::Release);
    let selector: u16;
    // SAFETY: ler `CS` não tem efeito colateral.
    unsafe { asm!("mov {0:x}, cs", out(reg) selector, options(nomem, nostack, preserves_flags)) };
    let stubs = ptr::addr_of!(rustos_isr_stubs).addr() as u64;
    let mut idt = [IdtEntry::MISSING; IDT_LEN];
    for (vector, entry) in (0..=u8::MAX).zip(idt.iter_mut()) {
        *entry = IdtEntry::interrupt_in(stubs + 16 * u64::from(vector), 0, selector);
    }
    // SAFETY: única escrita da IDT, com a CPU de boot sozinha e IF zerado.
    unsafe { IDT.0.get().write(idt) };
    // SAFETY: IDT completa e estática, com a GDT do bootloader ainda carregada.
    unsafe { load_idt() };
}

/// Carrega a GDT, a TSS e a IDT (já montada por `install`) numa CPU secundária.
///
/// # Safety
/// Ring 0 com IF zerado, depois de `install`. Os topos de `ist` e `tables` são só desta CPU.
pub unsafe fn load_cpu(ist: IstStacks, tables: &'static mut CpuTables) {
    tables.tss.ist[0] = ist.double_fault.as_u64();
    tables.tss.ist[1] = ist.nmi.as_u64();
    tables.tss.ist[2] = ist.machine_check.as_u64();
    tables.tss.ist[3] = ist.debug.as_u64();
    let tss_base = ptr::from_ref(&tables.tss).addr() as u64;
    tables.gdt = build_gdt(tss_base);
    // SAFETY: ring 0; `IA32_GS_BASE` existe em todo x86_64.
    let gs_base = unsafe { rdmsr(MSR_GS_BASE) };
    // SAFETY: GDT e TSS completas e estáticas, ring 0 (contrato de `load_cpu`).
    unsafe { load_gdt(ptr::from_ref(&tables.gdt).addr() as u64) };
    // SAFETY: carregar o seletor nulo em `GS` pode zerar a base; devolve a do bloco por CPU.
    unsafe { wrmsr(MSR_GS_BASE, gs_base) };
    prefix().tss_rsp0.store(
        ptr::addr_of!(tables.tss.rsp[0]).addr() as u64,
        Ordering::Relaxed,
    );
    // SAFETY: IDT completa e estática, com a GDT nova já carregada.
    unsafe { load_idt() };
    // SAFETY: ring 0, com o bloco por CPU em `GS` e a IDT carregada; o contrato é o desta função.
    unsafe { user::init_syscall() };
}

/// # Safety
/// `base` é a GDT completa e estática desta CPU, e a TSS que ela descreve é válida.
unsafe fn load_gdt(base: u64) {
    let pointer = TablePointer {
        limit: GDT_LIMIT,
        base,
    };
    // SAFETY: o ponteiro descreve a GDT estática, pelo contrato da função.
    unsafe {
        asm!("lgdt [{}]", in(reg) &raw const pointer, options(readonly, nostack, preserves_flags));
    };
    // SAFETY: a GDT tem código e dados do kernel nesses seletores; o retorno
    // distante recarrega o CS, e DS/ES/SS recebem o seletor de dados.
    unsafe {
        asm!(
            "push {cs}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            "mov ds, {data:x}",
            "mov es, {data:x}",
            "mov ss, {data:x}",
            "xor {tmp:e}, {tmp:e}",
            "mov fs, {tmp:x}",
            "mov gs, {tmp:x}",
            cs = in(reg) u64::from(KERNEL_CODE),
            data = in(reg) u64::from(KERNEL_DATA),
            tmp = out(reg) _,
            options(preserves_flags),
        );
    }
    // SAFETY: o seletor aponta para o descritor de TSS disponível da GDT.
    unsafe { asm!("ltr {0:x}", in(reg) TSS_SELECTOR, options(nostack, preserves_flags)) };
}

/// # Safety
/// `IDT` está completa e a GDT do kernel já está carregada.
unsafe fn load_idt() {
    let pointer = TablePointer {
        limit: IDT_LIMIT,
        base: IDT.0.get().addr() as u64,
    };
    // SAFETY: o ponteiro descreve a IDT estática, pelo contrato da função.
    unsafe {
        asm!("lidt [{}]", in(reg) &raw const pointer, options(readonly, nostack, preserves_flags));
    };
}

/// Dispara `int3`: a exceção volta ao fim da instrução.
///
/// # Safety
/// `install` já rodou: sem IDT, a exceção reinicia a máquina.
pub unsafe fn breakpoint() {
    // SAFETY: a IDT instalada trata o #BP e volta, pelo contrato da função.
    unsafe { asm!("int3", options(nomem, nostack)) };
}
