//! Controlador de interrupções GICv3 da ARM: a parte por MMIO (distribuidor e
//! redistribuidor). A interface da CPU é por registradores de sistema e mora em `arch/`.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

use core::hint::spin_loop;

use rustos_hal::RegisterBus;

mod regs;

use regs::{distributor as gicd, redistributor as gicr};

/// Primeiro INTID de PPI e de SPI; abaixo de 16 são SGIs.
const FIRST_PPI: u32 = 16;
const FIRST_SPI: u32 = 32;
/// Do 1020 em diante os INTIDs são especiais (espúrio), não interrupções.
const LAST_SPI: u32 = 1019;
/// Voltas de leitura que um `wait` espera antes de desistir.
const POLL_LIMIT: u32 = 1_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GicError {
    /// O `PIDR2` do distribuidor não anuncia GICv3 nem GICv4.
    NotGicV3,
    /// O INTID não existe neste GIC ou não é do tipo pedido (PPI/SPI).
    InvalidIntid,
    /// O redistribuidor mapeado não é o da CPU pedida.
    WrongRedistributor,
    /// O hardware não terminou a escrita (`RWP`) ou o despertar (`ChildrenAsleep`).
    Timeout,
}

/// Afinidade `Aff3.Aff2.Aff1.Aff0` de um `MPIDR_EL1`, no formato de `GICR_TYPER[63:32]`.
#[allow(
    clippy::cast_possible_truncation,
    reason = "as máscaras deixam 32 bits"
)]
pub const fn affinity_from_mpidr(mpidr: u64) -> u32 {
    ((mpidr & 0x00ff_ffff) | ((mpidr >> 8) & 0xff00_0000)) as u32
}

/// Valor de `GICD_IROUTER` que entrega a SPI à CPU de afinidade `affinity` (`IRM = 0`).
pub const fn router_for(affinity: u32) -> u64 {
    ((affinity as u64 & 0xff00_0000) << 8) | (affinity as u64 & 0x00ff_ffff)
}

/// `distributor` cobre a página de 64 KiB do GICD; `redistributor`, os dois
/// quadros de 64 KiB (`RD_base` e `SGI_base`) do redistribuidor de uma CPU.
#[derive(Debug)]
pub struct GicV3<D, R> {
    distributor: D,
    redistributor: R,
}

impl<D: RegisterBus, R: RegisterBus> GicV3<D, R> {
    pub const fn new(distributor: D, redistributor: R) -> Self {
        Self {
            distributor,
            redistributor,
        }
    }

    /// Desliga e religa o distribuidor com roteamento por afinidade (`ARE`) e
    /// grupo 1; deixa todas as SPIs desabilitadas.
    pub fn init_distributor(&mut self) -> Result<(), GicError> {
        let arch = self.distributor.read32(gicd::PIDR2) & gicd::PIDR2_ARCH_MASK;
        if arch != gicd::PIDR2_ARCH_V3 && arch != gicd::PIDR2_ARCH_V4 {
            return Err(GicError::NotGicV3);
        }
        self.distributor.write32(gicd::CTLR, 0);
        self.wait_distributor()?;
        for word in 1..=self.spi_words() {
            self.distributor
                .write32(gicd::ICENABLER + word as usize * 4, u32::MAX);
        }
        self.wait_distributor()?;
        self.distributor.write32(gicd::CTLR, gicd::CTLR_ENABLE);
        self.wait_distributor()
    }

    /// Acorda o redistribuidor da CPU de afinidade `affinity` e deixa SGIs e
    /// PPIs desabilitadas, no grupo 1.
    pub fn init_redistributor(&mut self, affinity: u32) -> Result<(), GicError> {
        let typer = self.redistributor.read64(gicr::TYPER);
        if typer >> gicr::TYPER_AFFINITY_SHIFT != u64::from(affinity) {
            return Err(GicError::WrongRedistributor);
        }
        let waker = self.redistributor.read32(gicr::WAKER);
        self.redistributor
            .write32(gicr::WAKER, waker & !gicr::WAKER_PROCESSOR_SLEEP);
        self.poll(|this| {
            this.redistributor.read32(gicr::WAKER) & gicr::WAKER_CHILDREN_ASLEEP == 0
        })?;
        self.redistributor
            .write32(gicr::SGI_BASE + gicd::ICENABLER, u32::MAX);
        self.redistributor
            .write32(gicr::SGI_BASE + gicd::IGROUPR, u32::MAX);
        self.wait_redistributor()
    }

    /// Habilita uma PPI (16–31) no redistribuidor, sensível a nível, no grupo 1.
    pub fn enable_ppi(&mut self, intid: u32, priority: u8) -> Result<(), GicError> {
        if !(FIRST_PPI..FIRST_SPI).contains(&intid) {
            return Err(GicError::InvalidIntid);
        }
        let bus = &self.redistributor;
        let base = gicr::SGI_BASE;
        let bit = 1 << intid;
        update(bus, base + gicd::IGROUPR, |word| word | bit);
        bus.write8(base + gicd::IPRIORITYR + intid as usize, priority);
        set_level_sensitive(bus, base + gicd::ICFGR, intid);
        bus.write32(base + gicd::ICPENDR, bit);
        bus.write32(base + gicd::ISENABLER, bit);
        Ok(())
    }

    /// Habilita uma SPI no distribuidor, sensível a nível, no grupo 1, roteada
    /// para `router` (ver `router_for`).
    pub fn enable_spi(&mut self, intid: u32, priority: u8, router: u64) -> Result<(), GicError> {
        let last = (self.spi_words() * 32 + 31).min(LAST_SPI);
        if !(FIRST_SPI..=last).contains(&intid) {
            return Err(GicError::InvalidIntid);
        }
        let bus = &self.distributor;
        let word = (intid / 32) as usize * 4;
        let bit = 1 << (intid % 32);
        update(bus, gicd::IGROUPR + word, |value| value | bit);
        bus.write8(gicd::IPRIORITYR + intid as usize, priority);
        set_level_sensitive(bus, gicd::ICFGR, intid);
        bus.write64(gicd::IROUTER + intid as usize * 8, router);
        bus.write32(gicd::ICPENDR + word, bit);
        bus.write32(gicd::ISENABLER + word, bit);
        Ok(())
    }

    /// Quantas palavras de 32 INTIDs de SPI o GIC tem, além da palavra 0 (SGI/PPI).
    fn spi_words(&self) -> u32 {
        self.distributor.read32(gicd::TYPER) & gicd::TYPER_LINES_MASK
    }

    fn wait_distributor(&self) -> Result<(), GicError> {
        self.poll(|this| this.distributor.read32(gicd::CTLR) & gicd::CTLR_RWP == 0)
    }

    fn wait_redistributor(&self) -> Result<(), GicError> {
        self.poll(|this| this.redistributor.read32(gicr::CTLR) & gicr::CTLR_RWP == 0)
    }

    fn poll(&self, done: impl Fn(&Self) -> bool) -> Result<(), GicError> {
        for _ in 0..POLL_LIMIT {
            if done(self) {
                return Ok(());
            }
            spin_loop();
        }
        Err(GicError::Timeout)
    }
}

fn update(bus: &impl RegisterBus, offset: usize, change: impl FnOnce(u32) -> u32) {
    bus.write32(offset, change(bus.read32(offset)));
}

/// `ICFGR` tem 2 bits por INTID; o bit alto é 1 para borda e 0 para nível.
fn set_level_sensitive(bus: &impl RegisterBus, base: usize, intid: u32) {
    let offset = base + (intid / 16) as usize * 4;
    let edge_bit = 1 << ((intid % 16) * 2 + 1);
    update(bus, offset, |word| word & !edge_bit);
}

#[cfg(test)]
mod tests;
