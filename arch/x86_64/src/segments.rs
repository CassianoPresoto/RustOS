//! Formato da GDT, da TSS e dos gates da IDT. Puro: a carga nos registradores
//! mora em `traps`.

/// Seletores da GDT do kernel. Dados do usuário antes do código: é a ordem que
/// o `sysret` espera (fase 4).
pub const KERNEL_CODE: u16 = 0x08;
pub const KERNEL_DATA: u16 = 0x10;
pub const USER_DATA: u16 = 0x18 | 3;
pub const USER_CODE: u16 = 0x20 | 3;
pub const TSS_SELECTOR: u16 = 0x28;

/// Nula, código e dados do kernel, dados e código do usuário, e a TSS (2 entradas).
pub const GDT_LEN: usize = 7;
const TSS_INDEX: usize = 5;
/// Limite (tamanho menos 1) que `lgdt` e `lidt` recebem.
pub const GDT_LIMIT: u16 = 7 * 8 - 1;
pub const IDT_LIMIT: u16 = 256 * 16 - 1;

const ACCESS_PRESENT_SEGMENT: u64 = 0x90;
const ACCESS_ACCESSED: u64 = 0x01;
const ACCESS_CODE: u64 = 0x08;
const ACCESS_READ_WRITE: u64 = 0x02;
const ACCESS_RING3: u64 = 0x60;
/// Granularidade de 4 KiB com limite 0xFFFFF; `L` no código, `D/B` nos dados.
const FLAGS_CODE64: u64 = 0xA;
const FLAGS_DATA: u64 = 0xC;
/// TSS de 64 bits disponível (tipo 9), presente.
const ACCESS_TSS: u64 = 0x89;

const fn segment(access: u64, flags: u64) -> u64 {
    0xFFFF | (access << 40) | (0xF << 48) | (flags << 52)
}

const KERNEL_CODE_DESC: u64 = segment(
    ACCESS_PRESENT_SEGMENT | ACCESS_ACCESSED | ACCESS_CODE | ACCESS_READ_WRITE,
    FLAGS_CODE64,
);
const KERNEL_DATA_DESC: u64 = segment(
    ACCESS_PRESENT_SEGMENT | ACCESS_ACCESSED | ACCESS_READ_WRITE,
    FLAGS_DATA,
);

/// Descritor de TSS de 16 bytes: devolve (baixo, alto).
pub const fn tss_descriptor(base: u64, limit: u32) -> (u64, u64) {
    let low = (limit as u64 & 0xFFFF)
        | ((base & 0xFF_FFFF) << 16)
        | (ACCESS_TSS << 40)
        | (((limit as u64 >> 16) & 0xF) << 48)
        | ((base >> 24 & 0xFF) << 56);
    (low, base >> 32)
}

/// A GDT inteira, com a TSS em `tss_base`.
pub const fn build_gdt(tss_base: u64) -> [u64; GDT_LEN] {
    let (tss_low, tss_high) = tss_descriptor(tss_base, TSS_SIZE as u32 - 1);
    let mut gdt = [0; GDT_LEN];
    gdt[1] = KERNEL_CODE_DESC;
    gdt[2] = KERNEL_DATA_DESC;
    gdt[3] = KERNEL_DATA_DESC | (ACCESS_RING3 << 40);
    gdt[4] = KERNEL_CODE_DESC | (ACCESS_RING3 << 40);
    gdt[TSS_INDEX] = tss_low;
    gdt[TSS_INDEX + 1] = tss_high;
    gdt
}

/// TSS de 64 bits (SDM vol. 3A, figura 8-11). `ist[0]` é a IST1.
#[derive(Clone, Copy, Debug)]
#[repr(C, packed(4))]
pub struct Tss {
    reserved0: u32,
    pub rsp: [u64; 3],
    reserved1: u64,
    pub ist: [u64; 7],
    reserved2: u64,
    reserved3: u16,
    pub iomap_base: u16,
}

/// Tamanho da TSS de 64 bits; o teste de layout confere contra `size_of`.
pub const TSS_SIZE: u16 = 104;

impl Tss {
    /// Sem bitmap de E/S: `iomap_base` além do limite nega toda porta ao ring 3.
    pub const fn new() -> Self {
        Self {
            reserved0: 0,
            rsp: [0; 3],
            reserved1: 0,
            ist: [0; 7],
            reserved2: 0,
            reserved3: 0,
            iomap_base: TSS_SIZE,
        }
    }
}

impl Default for Tss {
    fn default() -> Self {
        Self::new()
    }
}

/// Gate de interrupção (interrupções mascaradas na entrada), presente, DPL 0.
const GATE_INTERRUPT: u8 = 0x8E;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    attributes: u8,
    offset_mid: u16,
    offset_high: u32,
    reserved: u32,
}

pub const IDT_LEN: usize = 256;

impl IdtEntry {
    pub const MISSING: Self = Self {
        offset_low: 0,
        selector: 0,
        ist: 0,
        attributes: 0,
        offset_mid: 0,
        offset_high: 0,
        reserved: 0,
    };

    /// `ist` é o número da IST (1 a 7), ou 0 para ficar na pilha atual.
    pub const fn interrupt(handler: u64, ist: u8) -> Self {
        Self::interrupt_in(handler, ist, KERNEL_CODE)
    }

    /// Como `interrupt`, com o seletor de código de uma GDT que não é a do kernel.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "o endereço é fatiado de propósito"
    )]
    pub const fn interrupt_in(handler: u64, ist: u8, selector: u16) -> Self {
        Self {
            offset_low: handler as u16,
            selector,
            ist,
            attributes: GATE_INTERRUPT,
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            reserved: 0,
        }
    }

    pub const fn handler(self) -> u64 {
        self.offset_low as u64 | (self.offset_mid as u64) << 16 | (self.offset_high as u64) << 32
    }
}

/// Operando de `lgdt` e `lidt`.
#[derive(Clone, Copy, Debug)]
#[repr(C, packed)]
pub struct TablePointer {
    pub limit: u16,
    pub base: u64,
}

#[cfg(test)]
mod tests {
    use core::mem::{offset_of, size_of};

    use super::*;

    #[test]
    fn tss_layout_matches_the_manual() {
        assert_eq!(size_of::<Tss>(), usize::from(TSS_SIZE));
        assert_eq!(offset_of!(Tss, rsp), 0x04);
        assert_eq!(offset_of!(Tss, ist), 0x24);
        assert_eq!(offset_of!(Tss, iomap_base), 0x66);
        assert_eq!(Tss::new().iomap_base, 104);
    }

    #[test]
    fn idt_entry_is_16_bytes_with_the_manual_offsets() {
        assert_eq!(size_of::<IdtEntry>(), 16);
        assert_eq!(offset_of!(IdtEntry, selector), 2);
        assert_eq!(offset_of!(IdtEntry, ist), 4);
        assert_eq!(offset_of!(IdtEntry, attributes), 5);
        assert_eq!(offset_of!(IdtEntry, offset_mid), 6);
        assert_eq!(offset_of!(IdtEntry, offset_high), 8);
        assert_eq!(size_of::<TablePointer>(), 10);
    }

    #[test]
    fn gate_splits_the_handler_address_in_three() {
        let entry = IdtEntry::interrupt(0xffff_ffff_8123_4567, 2);
        assert_eq!(entry.handler(), 0xffff_ffff_8123_4567);
        assert_eq!(entry.ist, 2);
        assert_eq!(entry.attributes, 0x8E);
        assert_eq!(entry.selector, KERNEL_CODE);
    }

    #[test]
    fn kernel_code_is_a_64_bit_ring0_segment() {
        let gdt = build_gdt(0);
        assert_eq!(gdt[0], 0);
        assert_eq!(gdt[1], 0x00AF_9B00_0000_FFFF);
        assert_eq!(gdt[2], 0x00CF_9300_0000_FFFF);
    }

    #[test]
    fn user_segments_are_ring3_and_ordered_for_sysret() {
        let gdt = build_gdt(0);
        assert_eq!(gdt[3], 0x00CF_F300_0000_FFFF);
        assert_eq!(gdt[4], 0x00AF_FB00_0000_FFFF);
        assert_eq!(usize::from(USER_DATA >> 3) + 1, usize::from(USER_CODE >> 3));
    }

    #[test]
    fn table_limits_cover_exactly_the_tables() {
        assert_eq!(usize::from(GDT_LIMIT) + 1, GDT_LEN * size_of::<u64>());
        assert_eq!(usize::from(IDT_LIMIT) + 1, IDT_LEN * size_of::<IdtEntry>());
    }

    #[test]
    fn selectors_index_the_expected_entries() {
        assert_eq!(usize::from(TSS_SELECTOR >> 3), TSS_INDEX);
        assert_eq!(usize::from(KERNEL_DATA >> 3), 2);
    }

    #[test]
    fn tss_descriptor_encodes_base_and_limit() {
        let (low, high) = tss_descriptor(0xffff_ffff_8040_2000, 103);
        assert_eq!(low & 0xFFFF, 103);
        assert_eq!((low >> 16) & 0xFF_FFFF, 0x40_2000);
        assert_eq!((low >> 40) & 0xFF, 0x89);
        assert_eq!(low >> 56, 0x80);
        assert_eq!(high, 0xffff_ffff);
    }

    #[test]
    fn gdt_places_the_tss_in_two_consecutive_slots() {
        let gdt = build_gdt(0x1234_5000);
        let (low, high) = tss_descriptor(0x1234_5000, 103);
        assert_eq!((gdt[5], gdt[6]), (low, high));
    }
}
