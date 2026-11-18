//! Deslocamentos e bits dos registradores do GICv3 (Arm IHI 0069), conferidos
//! contra `include/linux/irqchip/arm-gic-v3.h`.

pub mod distributor {
    pub const CTLR: usize = 0x0000;
    pub const TYPER: usize = 0x0004;
    pub const IGROUPR: usize = 0x0080;
    pub const ISENABLER: usize = 0x0100;
    pub const ICENABLER: usize = 0x0180;
    pub const ICPENDR: usize = 0x0280;
    pub const IPRIORITYR: usize = 0x0400;
    pub const ICFGR: usize = 0x0C00;
    pub const IROUTER: usize = 0x6000;
    pub const PIDR2: usize = 0xFFE8;

    /// `EnableGrp0 | EnableGrp1 | ARE`: o que o Linux escreve (`ENABLE_G1 | ENABLE_G1A | ARE_NS`).
    pub const CTLR_ENABLE: u32 = (1 << 0) | (1 << 1) | (1 << 4);
    /// Escrita pendente no registrador de controle (`RWP`).
    pub const CTLR_RWP: u32 = 1 << 31;
    /// `ITLinesNumber`: o GIC tem `32 * (N + 1)` INTIDs.
    pub const TYPER_LINES_MASK: u32 = 0x1f;
    pub const PIDR2_ARCH_MASK: u32 = 0xf0;
    pub const PIDR2_ARCH_V3: u32 = 0x30;
    pub const PIDR2_ARCH_V4: u32 = 0x40;
}

pub mod redistributor {
    pub const CTLR: usize = 0x0000;
    pub const TYPER: usize = 0x0008;
    pub const WAKER: usize = 0x0014;
    /// Quadro `SGI_base`, 64 KiB depois do `RD_base`; seus registradores repetem
    /// os `IGROUPR0`, `ISENABLER0`… do distribuidor.
    pub const SGI_BASE: usize = 0x1_0000;

    pub const CTLR_RWP: u32 = 1 << 3;
    pub const TYPER_AFFINITY_SHIFT: u32 = 32;
    /// `GICR_TYPER.VLPIS` (bit 1): o frame é de GICv4, com `VLPI_base` e uma página reservada
    /// a mais (4 quadros de 64 KiB em vez de 2).
    pub const TYPER_VLPIS: u64 = 1 << 1;
    /// `GICR_TYPER.Last` (bit 4): é o último redistribuidor da região.
    pub const TYPER_LAST: u64 = 1 << 4;
    /// Tamanho de um redistribuidor GICv3 (`RD_base` e `SGI_base`).
    pub const FRAME_SIZE: usize = 0x2_0000;
    pub const WAKER_PROCESSOR_SLEEP: u32 = 1 << 1;
    pub const WAKER_CHILDREN_ASLEEP: u32 = 1 << 2;
}
