//! GDT, TSS e IDT próprias, os stubs de entrada e o despacho para os `Handlers`.

use core::arch::{asm, global_asm};
use core::cell::UnsafeCell;
use core::hint::spin_loop;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use rustos_hal::{Handlers, Irq, VirtAddr};

use crate::apic;
use crate::fault::{
    self, DOUBLE_FAULT, ERROR_CODE_VECTORS, FIRST_IRQ_VECTOR, MACHINE_CHECK, NMI, PAGE_FAULT,
};
use crate::lapic::VECTOR_SPURIOUS;
use crate::segments::{
    GDT_LEN, GDT_LIMIT, IDT_LEN, IDT_LIMIT, IdtEntry, KERNEL_CODE, KERNEL_DATA, TSS_SELECTOR,
    TablePointer, Tss, build_gdt,
};

/// Topos das pilhas de #DF, NMI e #MC (IST1 a IST3), com página de guarda abaixo.
#[derive(Clone, Copy, Debug)]
pub struct IstStacks {
    pub double_fault: VirtAddr,
    pub nmi: VirtAddr,
    pub machine_check: VirtAddr,
}

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

static GDT: Table<[u64; GDT_LEN]> = Table(UnsafeCell::new([0; GDT_LEN]));
static TSS: Table<Tss> = Table(UnsafeCell::new(Tss::new()));
static IDT: Table<[IdtEntry; IDT_LEN]> = Table(UnsafeCell::new([IdtEntry::MISSING; IDT_LEN]));
static HANDLERS: AtomicPtr<Handlers> = AtomicPtr::new(ptr::null_mut());

// Cada stub ocupa 16 bytes: o gate do vetor `v` aponta para `stubs + 16 * v`.
// A entrada comum empilha os registradores na ordem de `TrapFrame`.
global_asm!(
    r#"
    .section .text.rustos_isr, "ax"
    .balign 16
    .global rustos_isr_stubs
rustos_isr_stubs:
    .set vec, 0
    .rept 256
    .balign 16
    .if vec < 32 && (({ERROR_CODE_VECTORS} >> vec) & 1) != 0
    .else
    push 0
    .endif
    push vec
    jmp rustos_isr_common
    .set vec, vec + 1
    .endr

rustos_isr_common:
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
    "#,
    ERROR_CODE_VECTORS = const ERROR_CODE_VECTORS,
    dispatch = sym trap_dispatch,
);

unsafe extern "C" {
    static rustos_isr_stubs: u8;
}

extern "C" fn trap_dispatch(frame: &mut TrapFrame) {
    let vector = u8::try_from(frame.vector).unwrap_or(u8::MAX);
    // SAFETY: `install` guardou um `&'static Handlers` antes de carregar a IDT.
    let Some(handlers) = (unsafe { HANDLERS.load(Ordering::Acquire).as_ref() }) else {
        halt();
    };
    if vector < FIRST_IRQ_VECTOR {
        let cr2 = if vector == PAGE_FAULT { read_cr2() } else { 0 };
        let report = fault::decode(vector, frame.error, frame.rip, cr2);
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

/// IST de cada vetor que precisa de pilha própria: #DF, NMI e #MC.
const fn ist_for(vector: u8) -> u8 {
    match vector {
        DOUBLE_FAULT => 1,
        NMI => 2,
        MACHINE_CHECK => 3,
        _ => 0,
    }
}

/// Carrega a GDT, a TSS e a IDT e despacha para `handlers`.
///
/// # Safety
/// Ring 0, uma vez, com IF zerado; os topos de `ist` são de pilhas mapeadas e só das exceções.
pub unsafe fn install(ist: IstStacks, handlers: &'static Handlers) {
    HANDLERS.store(ptr::from_ref(handlers).cast_mut(), Ordering::Release);
    let tss_base = ptr::from_ref(&TSS).addr() as u64;
    // SAFETY: única escrita, ainda sem a TSS carregada (contrato de `install`).
    unsafe {
        let tss = &mut *TSS.0.get();
        tss.ist[0] = ist.double_fault.as_u64();
        tss.ist[1] = ist.nmi.as_u64();
        tss.ist[2] = ist.machine_check.as_u64();
    }
    // SAFETY: única escrita, antes do `lgdt` (contrato de `install`).
    unsafe { GDT.0.get().write(build_gdt(tss_base)) };
    let stubs = ptr::addr_of!(rustos_isr_stubs).addr() as u64;
    let mut idt = [IdtEntry::MISSING; IDT_LEN];
    for (vector, entry) in (0..=u8::MAX).zip(idt.iter_mut()) {
        *entry = IdtEntry::interrupt(stubs + 16 * u64::from(vector), ist_for(vector));
    }
    // SAFETY: única escrita, antes do `lidt` (contrato de `install`).
    unsafe { IDT.0.get().write(idt) };
    // SAFETY: GDT e TSS completas e estáticas, ring 0 (contrato de `install`).
    unsafe { load_gdt() };
    // SAFETY: IDT completa e estática, com a GDT nova já carregada.
    unsafe { load_idt() };
}

/// # Safety
/// `GDT` está completa e a TSS que ela descreve é válida.
unsafe fn load_gdt() {
    let pointer = TablePointer {
        limit: GDT_LIMIT,
        base: GDT.0.get().addr() as u64,
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
