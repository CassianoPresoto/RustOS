use core::cell::RefCell;
use std::collections::BTreeMap;

use super::regs::{distributor as gicd, redistributor as gicr};
use super::*;

/// Registradores como memória, mais o registro das escritas na ordem em que vieram.
#[derive(Default)]
struct FakeBus {
    registers: RefCell<BTreeMap<usize, u32>>,
    writes: RefCell<Vec<(usize, u32)>>,
    /// Imita um `ChildrenAsleep` que nunca desce e um `RWP` que nunca cai.
    stuck: bool,
}

impl FakeBus {
    fn with(values: &[(usize, u32)]) -> Self {
        let bus = Self::default();
        bus.registers.borrow_mut().extend(values.iter().copied());
        bus
    }

    fn written(&self) -> Vec<(usize, u32)> {
        self.writes.borrow().clone()
    }

    fn value(&self, offset: usize) -> u32 {
        self.registers.borrow().get(&offset).copied().unwrap_or(0)
    }

    fn position(&self, offset: usize) -> usize {
        self.written()
            .iter()
            .position(|&(at, _)| at == offset)
            .expect("registrador não escrito")
    }
}

impl RegisterBus for FakeBus {
    fn read8(&self, offset: usize) -> u8 {
        self.value(offset & !3).to_le_bytes()[offset & 3]
    }

    fn write8(&self, offset: usize, value: u8) {
        let mut bytes = self.value(offset & !3).to_le_bytes();
        bytes[offset & 3] = value;
        self.registers
            .borrow_mut()
            .insert(offset & !3, u32::from_le_bytes(bytes));
        self.writes.borrow_mut().push((offset, u32::from(value)));
    }

    fn read32(&self, offset: usize) -> u32 {
        let value = self.value(offset);
        if self.stuck && offset == gicr::WAKER {
            return value | gicr::WAKER_CHILDREN_ASLEEP;
        }
        if self.stuck && offset == gicd::CTLR {
            return value | gicd::CTLR_RWP;
        }
        value
    }

    fn write32(&self, offset: usize, value: u32) {
        self.registers.borrow_mut().insert(offset, value);
        self.writes.borrow_mut().push((offset, value));
    }
}

fn distributor(lines: u32) -> FakeBus {
    FakeBus::with(&[(gicd::PIDR2, gicd::PIDR2_ARCH_V3), (gicd::TYPER, lines)])
}

fn redistributor(affinity: u32) -> FakeBus {
    FakeBus::with(&[
        (gicr::TYPER, 0),
        (gicr::TYPER + 4, affinity),
        (
            gicr::WAKER,
            gicr::WAKER_PROCESSOR_SLEEP | gicr::WAKER_CHILDREN_ASLEEP,
        ),
    ])
}

#[test]
fn distributor_is_disabled_before_it_is_reconfigured_and_enabled_last() {
    let mut gic = GicV3::new(distributor(2), redistributor(0));
    gic.init_distributor().unwrap();
    let writes = gic.distributor.written();
    assert_eq!(writes.first(), Some(&(gicd::CTLR, 0)));
    assert_eq!(writes.last(), Some(&(gicd::CTLR, gicd::CTLR_ENABLE)));
}

#[test]
fn distributor_masks_every_spi_word_the_gic_has() {
    let mut gic = GicV3::new(distributor(2), redistributor(0));
    gic.init_distributor().unwrap();
    for word in 1..=2 {
        assert_eq!(gic.distributor.value(gicd::ICENABLER + word * 4), u32::MAX);
    }
    assert_eq!(gic.distributor.value(gicd::ICENABLER + 12), 0);
    assert_eq!(gic.distributor.value(gicd::ICENABLER), 0);
}

#[test]
fn distributor_rejects_a_gic_that_is_not_v3() {
    let bus = FakeBus::with(&[(gicd::PIDR2, 0x20)]);
    let mut gic = GicV3::new(bus, redistributor(0));
    assert_eq!(gic.init_distributor(), Err(GicError::NotGicV3));
    assert_eq!(gic.distributor.written().len(), 0);
}

#[test]
fn distributor_gives_up_when_the_write_never_completes() {
    let mut bus = distributor(0);
    bus.stuck = true;
    let mut gic = GicV3::new(bus, redistributor(0));
    assert_eq!(gic.init_distributor(), Err(GicError::Timeout));
}

#[test]
fn redistributor_wakes_the_processor_and_masks_sgis_and_ppis() {
    let mut gic = GicV3::new(distributor(0), redistributor(0));
    // O fake não desce `ChildrenAsleep` sozinho: faz o papel do hardware.
    gic.redistributor
        .registers
        .borrow_mut()
        .insert(gicr::WAKER, gicr::WAKER_PROCESSOR_SLEEP);
    gic.init_redistributor(0).unwrap();
    assert_eq!(
        gic.redistributor.value(gicr::WAKER) & gicr::WAKER_PROCESSOR_SLEEP,
        0
    );
    assert_eq!(
        gic.redistributor.value(gicr::SGI_BASE + gicd::ICENABLER),
        u32::MAX
    );
    assert_eq!(
        gic.redistributor.value(gicr::SGI_BASE + gicd::IGROUPR),
        u32::MAX
    );
}

#[test]
fn redistributor_of_another_cpu_is_refused_before_any_write() {
    let mut gic = GicV3::new(distributor(0), redistributor(0x0000_0100));
    assert_eq!(gic.init_redistributor(0), Err(GicError::WrongRedistributor));
    assert_eq!(gic.redistributor.written().len(), 0);
}

#[test]
fn redistributor_that_never_wakes_times_out() {
    let mut bus = redistributor(0);
    bus.stuck = true;
    let mut gic = GicV3::new(distributor(0), bus);
    assert_eq!(gic.init_redistributor(0), Err(GicError::Timeout));
}

#[test]
fn ppi_is_configured_before_it_is_enabled() {
    let mut gic = GicV3::new(distributor(0), redistributor(0));
    gic.enable_ppi(27, 0x80).unwrap();
    let sgi = gicr::SGI_BASE;
    let bus = &gic.redistributor;
    assert_eq!(bus.value(sgi + gicd::IGROUPR), 1 << 27);
    assert_eq!(bus.read8(sgi + gicd::IPRIORITYR + 27), 0x80);
    assert_eq!(bus.value(sgi + gicd::ISENABLER), 1 << 27);
    let enable = bus.position(sgi + gicd::ISENABLER);
    assert!(bus.position(sgi + gicd::IGROUPR) < enable);
    assert!(bus.position(sgi + gicd::IPRIORITYR + 27) < enable);
}

#[test]
fn ppi_keeps_the_other_group_bits_and_clears_only_its_edge_bit() {
    let mut gic = GicV3::new(distributor(0), redistributor(0));
    let sgi = gicr::SGI_BASE;
    gic.redistributor.registers.borrow_mut().extend([
        (sgi + gicd::IGROUPR, 1 << 3),
        (sgi + gicd::ICFGR + 4, u32::MAX),
    ]);
    gic.enable_ppi(27, 0x80).unwrap();
    assert_eq!(
        gic.redistributor.value(sgi + gicd::IGROUPR),
        (1 << 3) | (1 << 27)
    );
    // INTID 27: registrador 1 de `ICFGR`, campo 11, bit alto = bit 23.
    assert_eq!(gic.redistributor.value(sgi + gicd::ICFGR + 4), !(1 << 23));
}

#[test]
fn ppi_rejects_sgis_and_spis() {
    let mut gic = GicV3::new(distributor(0), redistributor(0));
    assert_eq!(gic.enable_ppi(15, 0), Err(GicError::InvalidIntid));
    assert_eq!(gic.enable_ppi(32, 0), Err(GicError::InvalidIntid));
    assert_eq!(gic.redistributor.written().len(), 0);
}

#[test]
fn spi_is_routed_and_configured_before_it_is_enabled() {
    let mut gic = GicV3::new(distributor(2), redistributor(0));
    gic.enable_spi(33, 0x80, router_for(0)).unwrap();
    let bus = &gic.distributor;
    assert_eq!(bus.value(gicd::IGROUPR + 4), 1 << 1);
    assert_eq!(bus.read8(gicd::IPRIORITYR + 33), 0x80);
    assert_eq!(bus.value(gicd::ISENABLER + 4), 1 << 1);
    let enable = bus.position(gicd::ISENABLER + 4);
    assert!(bus.position(gicd::IROUTER + 33 * 8) < enable);
    assert!(bus.position(gicd::IPRIORITYR + 33) < enable);
}

#[test]
fn spi_router_is_written_as_one_64_bit_value() {
    let mut gic = GicV3::new(distributor(2), redistributor(0));
    gic.enable_spi(40, 0, router_for(0x0100_0203)).unwrap();
    let router = gicd::IROUTER + 40 * 8;
    assert_eq!(gic.distributor.value(router), 0x0000_0203);
    assert_eq!(gic.distributor.value(router + 4), 0x01);
}

#[test]
fn spi_beyond_what_the_gic_has_is_rejected() {
    let mut gic = GicV3::new(distributor(1), redistributor(0));
    assert_eq!(gic.enable_spi(32, 0, 0), Ok(()));
    assert_eq!(gic.enable_spi(63, 0, 0), Ok(()));
    assert_eq!(gic.enable_spi(64, 0, 0), Err(GicError::InvalidIntid));
    assert_eq!(gic.enable_spi(31, 0, 0), Err(GicError::InvalidIntid));
}

#[test]
fn spi_never_reaches_the_special_intids() {
    let mut gic = GicV3::new(distributor(31), redistributor(0));
    assert_eq!(gic.enable_spi(1019, 0, 0), Ok(()));
    assert_eq!(gic.enable_spi(1020, 0, 0), Err(GicError::InvalidIntid));
}

#[test]
fn affinity_packs_aff3_aff2_aff1_aff0_from_mpidr() {
    let mpidr = 0x0000_0001_8004_0302_u64;
    assert_eq!(affinity_from_mpidr(mpidr), 0x0104_0302);
    assert_eq!(affinity_from_mpidr(0x8000_0000), 0);
}

#[test]
fn router_puts_aff3_in_bits_39_to_32_and_leaves_irm_clear() {
    assert_eq!(router_for(0x0104_0302), 0x01_0004_0302);
    assert_eq!(router_for(0) & (1 << 31), 0);
}

/// Uma região com um redistribuidor por afinidade em `frames`, cada um com `stride` bytes.
fn region(frames: &[(u32, u64)], stride: usize) -> FakeBus {
    let bus = FakeBus::default();
    for (index, &(affinity, flags)) in frames.iter().enumerate() {
        let at = index * stride + gicr::TYPER;
        let low = u32::try_from(flags).unwrap();
        bus.registers.borrow_mut().insert(at, low);
        bus.registers.borrow_mut().insert(at + 4, affinity);
    }
    bus
}

#[test]
fn finds_the_redistributor_of_a_cpu_by_walking_the_frames() {
    let bus = region(
        &[(0, 0), (1, 0), (2, 0), (3, gicr::TYPER_LAST)],
        gicr::FRAME_SIZE,
    );
    assert_eq!(find_redistributor(&bus, 0), Ok(0));
    assert_eq!(find_redistributor(&bus, 2), Ok(2 * gicr::FRAME_SIZE));
    assert_eq!(find_redistributor(&bus, 3), Ok(3 * gicr::FRAME_SIZE));
}

#[test]
fn stops_at_the_last_frame_when_the_affinity_is_not_there() {
    let bus = region(&[(0, 0), (1, gicr::TYPER_LAST)], gicr::FRAME_SIZE);
    assert_eq!(
        find_redistributor(&bus, 7),
        Err(GicError::WrongRedistributor)
    );
}

#[test]
fn gicv4_frames_are_twice_as_long() {
    let stride = 2 * gicr::FRAME_SIZE;
    let bus = region(
        &[
            (0, gicr::TYPER_VLPIS),
            (1, gicr::TYPER_VLPIS | gicr::TYPER_LAST),
        ],
        stride,
    );
    assert_eq!(find_redistributor(&bus, 1), Ok(stride));
}

#[test]
fn sgi_is_configured_before_it_is_enabled_and_only_sgis_are_accepted() {
    let mut gic = GicV3::new(distributor(0), redistributor(0));
    gic.enable_sgi(1, 0x20).unwrap();
    let sgi = gicr::SGI_BASE;
    let bus = &gic.redistributor;
    assert_eq!(bus.value(sgi + gicd::ISENABLER), 1 << 1);
    assert_eq!(bus.read8(sgi + gicd::IPRIORITYR + 1), 0x20);
    assert!(bus.position(sgi + gicd::IPRIORITYR + 1) < bus.position(sgi + gicd::ISENABLER));
    assert_eq!(gic.enable_sgi(16, 0), Err(GicError::InvalidIntid));
}
