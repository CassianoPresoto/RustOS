//! Acesso ao Local APIC da CPU atual: MSR no modo x2APIC, MMIO no xAPIC. A
//! sequência de uso mora em `lapic`.

use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use rustos_hal::{PhysAddr, RegisterBus, VirtAddr};

use crate::features::Features;
use crate::lapic::{
    APIC_BASE_ADDRESS, APIC_BASE_ENABLE, APIC_BASE_X2APIC, ApicRegisters, LocalApic, MSR_APIC_BASE,
    x2apic_msr,
};
use crate::mmio::MmioBus;
use crate::sysreg::{rdmsr, wrmsr};

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

/// Avisa o Local APIC que a interrupção em serviço terminou. Sem efeito antes de `publish`.
pub fn eoi() {
    if ACTIVE_X2APIC.load(Ordering::Acquire) {
        ApicAccess {
            x2apic: true,
            mmio: None,
        }
        .local()
        .eoi();
        return;
    }
    let base = ACTIVE_MMIO.load(Ordering::Relaxed);
    if base != 0 {
        let virt = VirtAddr::new(base as u64);
        // SAFETY: `publish` guardou a página mapeada que `enable_xapic` recebeu.
        let bus = unsafe { MmioBus::new(virt) };
        ApicAccess {
            x2apic: false,
            mmio: Some(bus),
        }
        .local()
        .eoi();
    }
}
