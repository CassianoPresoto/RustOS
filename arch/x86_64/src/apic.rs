//! Acesso ao Local APIC da CPU atual: MSR no modo x2APIC, MMIO no xAPIC. A
//! sequência de uso mora em `lapic`.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rustos_hal::{Ipi, IpiKind, PhysAddr, RegisterBus, VirtAddr};

use crate::features::Features;
use crate::lapic::{
    APIC_BASE_ADDRESS, APIC_BASE_ENABLE, APIC_BASE_X2APIC, ApicRegisters, ICR_HIGH, ICR_LOW,
    LocalApic, MSR_APIC_BASE, VECTOR_RESCHEDULE, VECTOR_STOP, x2apic_msr,
};
use crate::mmio::MmioBus;
use crate::sysreg::{MSR_TSC_DEADLINE, rdmsr, wrmsr};

/// Como o Local APIC desta CPU deve ser usado, pelo CPUID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApicMode {
    X2Apic,
    /// Registradores em MMIO; o chamador mapeia a página `phys` como dispositivo.
    XApic {
        phys: PhysAddr,
    },
}

/// O modo disponível, preferindo o x2APIC; `None` se a CPU não tem APIC.
pub fn detect(features: Features) -> Option<ApicMode> {
    if features.x2apic {
        return Some(ApicMode::X2Apic);
    }
    if !features.apic {
        return None;
    }
    // SAFETY: a CPU anuncia APIC, logo `IA32_APIC_BASE` existe; ring 0 pelo kernel.
    let base = unsafe { rdmsr(MSR_APIC_BASE) };
    Some(ApicMode::XApic {
        phys: PhysAddr::new(base & APIC_BASE_ADDRESS),
    })
}

/// Os registradores do Local APIC no modo ativo.
#[derive(Clone, Copy, Debug)]
pub struct ApicAccess {
    x2apic: bool,
    mmio: Option<MmioBus>,
}

impl ApicAccess {
    /// Liga o modo x2APIC.
    ///
    /// # Safety
    /// Ring 0, e o CPUID anuncia x2APIC. Depois, só o modo x2APIC vale.
    pub unsafe fn enable_x2apic() -> Self {
        // SAFETY: `IA32_APIC_BASE` existe, pelo contrato da função.
        let base = unsafe { rdmsr(MSR_APIC_BASE) };
        if base & APIC_BASE_ENABLE == 0 {
            // SAFETY: o x2APIC só se alcança passando pelo xAPIC habilitado (SDM 11.12.5).
            unsafe { wrmsr(MSR_APIC_BASE, base | APIC_BASE_ENABLE) };
        }
        // SAFETY: o APIC já está habilitado e o CPUID anuncia o x2APIC.
        unsafe { wrmsr(MSR_APIC_BASE, base | APIC_BASE_ENABLE | APIC_BASE_X2APIC) };
        Self {
            x2apic: true,
            mmio: None,
        }
    }

    /// Liga o modo xAPIC, com os registradores em `mmio`.
    ///
    /// # Safety
    /// Ring 0, o CPUID anuncia APIC, e `mmio` é a página de `IA32_APIC_BASE`.
    pub unsafe fn enable_xapic(mmio: VirtAddr) -> Self {
        // SAFETY: `IA32_APIC_BASE` existe, pelo contrato da função.
        let base = unsafe { rdmsr(MSR_APIC_BASE) };
        // SAFETY: liga o APIC no modo xAPIC (EXTD zerado), que a CPU anuncia.
        unsafe { wrmsr(MSR_APIC_BASE, (base | APIC_BASE_ENABLE) & !APIC_BASE_X2APIC) };
        Self {
            x2apic: false,
            // SAFETY: a página mapeada cobre todos os registradores, pelo contrato.
            mmio: Some(unsafe { MmioBus::new(mmio) }),
        }
    }

    pub const fn is_x2apic(&self) -> bool {
        self.x2apic
    }

    pub const fn local(self) -> LocalApic<Self> {
        LocalApic::new(self)
    }
}

impl ApicRegisters for ApicAccess {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "registrador do APIC tem 32 bits"
    )]
    fn read(&self, offset: usize) -> u32 {
        match self.mmio {
            // SAFETY: o x2APIC está ligado e o MSR existe, pelo contrato de `enable_x2apic`.
            None => unsafe { rdmsr(x2apic_msr(offset)) as u32 },
            Some(bus) => bus.read32(offset),
        }
    }

    fn write(&self, offset: usize, value: u32) {
        match self.mmio {
            // SAFETY: o x2APIC está ligado e o MSR existe, pelo contrato de `enable_x2apic`.
            None => unsafe { wrmsr(x2apic_msr(offset), u64::from(value)) },
            Some(bus) => bus.write32(offset, value),
        }
    }

    fn write_icr(&self, destination: u32, low: u32) {
        match self.mmio {
            // SAFETY: o x2APIC está ligado; o ICR é o MSR 0x830, de 64 bits, com o destino em 63:32.
            None => unsafe {
                wrmsr(
                    x2apic_msr(ICR_LOW),
                    u64::from(destination) << 32 | u64::from(low),
                );
            },
            Some(bus) => {
                bus.write32(ICR_HIGH, destination << 24);
                bus.write32(ICR_LOW, low);
            }
        }
    }

    fn icr_pending(&self) -> bool {
        self.mmio
            .is_some_and(|bus| bus.read32(ICR_LOW) & (1 << 12) != 0)
    }
}

static ACTIVE_X2APIC: AtomicBool = AtomicBool::new(false);
static ACTIVE_MMIO: AtomicUsize = AtomicUsize::new(0);

/// Publica o acesso para o EOI da entrada de interrupção.
#[allow(
    clippy::cast_possible_truncation,
    reason = "x86_64 tem usize de 64 bits"
)]
pub fn publish(access: ApicAccess, mmio: Option<VirtAddr>) {
    ACTIVE_MMIO.store(
        mmio.map_or(0, |virt| virt.as_u64() as usize),
        Ordering::Relaxed,
    );
    ACTIVE_X2APIC.store(access.is_x2apic(), Ordering::Release);
}

/// O Local APIC publicado por `publish`; `None` antes dele.
fn published() -> Option<LocalApic<ApicAccess>> {
    if ACTIVE_X2APIC.load(Ordering::Acquire) {
        return Some(
            ApicAccess {
                x2apic: true,
                mmio: None,
            }
            .local(),
        );
    }
    let base = ACTIVE_MMIO.load(Ordering::Relaxed);
    if base == 0 {
        return None;
    }
    let virt = VirtAddr::new(base as u64);
    // SAFETY: `publish` guardou a página mapeada que `enable_xapic` recebeu.
    let bus = unsafe { MmioBus::new(virt) };
    Some(
        ApicAccess {
            x2apic: false,
            mmio: Some(bus),
        }
        .local(),
    )
}

/// Avisa o Local APIC que a interrupção em serviço terminou. Sem efeito antes de `publish`.
pub fn eoi() {
    if let Some(local) = published() {
        local.eoi();
    }
}

/// Como o timer do Local APIC dispara.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerMode {
    /// Contador de um tiro, em ticks do contador do timer.
    OneShot,
    /// Comparador do TSC, pelo MSR `IA32_TSC_DEADLINE`.
    TscDeadline,
}

/// Põe o timer no modo pedido, desarmado. Sem efeito antes de `publish`.
pub fn set_timer_mode(mode: TimerMode) {
    let Some(local) = published() else { return };
    match mode {
        TimerMode::OneShot => local.set_oneshot(),
        TimerMode::TscDeadline => {
            local.set_tsc_deadline();
            // SAFETY: `mfence` não tem pré-condição; ordena o LVT antes da escrita do deadline.
            unsafe { asm!("mfence", options(nostack, preserves_flags)) };
        }
    }
}

/// Arma o modo de um tiro: dispara `count` ticks do contador depois de agora.
pub fn arm_oneshot(count: u32) {
    if let Some(local) = published() {
        local.arm_oneshot(count);
    }
}

/// Arma o modo TSC-deadline: dispara quando o TSC chegar a `tsc`.
///
/// # Safety
/// O timer está em `TimerMode::TscDeadline` e o CPUID anuncia o recurso.
pub unsafe fn arm_tsc_deadline(tsc: u64) {
    // SAFETY: o contrato da função garante o modo e o MSR.
    unsafe { wrmsr(MSR_TSC_DEADLINE, tsc) };
}

/// `hal::Ipi` sobre o ICR do Local APIC desta CPU, no modo que `publish` guardou.
#[derive(Clone, Copy, Debug)]
pub struct X86Ipi {
    _published: (),
}

impl X86Ipi {
    /// # Safety
    /// `publish` já rodou: antes dele não há APIC para escrever o ICR.
    pub const unsafe fn new() -> Self {
        Self { _published: () }
    }
}

const fn vector_of(kind: IpiKind) -> u8 {
    match kind {
        IpiKind::Reschedule => VECTOR_RESCHEDULE,
        IpiKind::Stop => VECTOR_STOP,
    }
}

impl Ipi for X86Ipi {
    fn send(&self, hw_id: u64, kind: IpiKind) {
        if let Some(local) = published() {
            local.send_ipi(u32::try_from(hw_id).unwrap_or(u32::MAX), vector_of(kind));
        }
    }

    fn send_all_but_self(&self, kind: IpiKind) {
        if let Some(local) = published() {
            local.send_ipi_all_but_self(vector_of(kind));
        }
    }
}
