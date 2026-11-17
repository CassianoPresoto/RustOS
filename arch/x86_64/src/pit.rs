//! PIT 8254, canal 2, como referência de tempo para calibrar o TSC e o timer do
//! APIC, mais a aritmética de calibração. Puro, sobre um `RegisterBus` na base 0x40.

use rustos_hal::RegisterBus;

/// Frequência do oscilador do PIT (QEMU `PIT_FREQ`, `include/hw/timer/i8254.h`).
pub const PIT_HZ: u64 = 1_193_182;

const CHANNEL2_DATA: usize = 0x02;
const COMMAND: usize = 0x03;
/// Porta 0x61 (controle do alto-falante e porta B do sistema), vista da base 0x40.
const PORT_B: usize = 0x21;
/// Canal 2, acesso baixo/alto, modo 0 (fim de contagem), binário.
const CHANNEL2_MODE0: u8 = 0xB0;
const GATE: u8 = 1 << 0;
const SPEAKER_DATA: u8 = 1 << 1;
const OUT2: u8 = 1 << 5;

#[derive(Debug)]
pub struct Pit<B> {
    bus: B,
}

impl<B: RegisterBus> Pit<B> {
    pub const fn new(bus: B) -> Self {
        Self { bus }
    }

    /// Dispara a contagem de `count` ticks sem som; `expired` vira verdadeiro no fim.
    pub fn arm(&self, count: u16) {
        let port_b = (self.bus.read8(PORT_B) & !SPEAKER_DATA) | GATE;
        self.bus.write8(PORT_B, port_b);
        self.bus.write8(COMMAND, CHANNEL2_MODE0);
        let [low, high] = count.to_le_bytes();
        self.bus.write8(CHANNEL2_DATA, low);
        self.bus.write8(CHANNEL2_DATA, high);
    }

    pub fn expired(&self) -> bool {
        self.bus.read8(PORT_B) & OUT2 != 0
    }

    /// Fecha o gate do canal 2.
    pub fn disarm(&self) {
        self.bus.write8(PORT_B, self.bus.read8(PORT_B) & !GATE);
    }
}

/// Duas leituras dos contadores, uma de cada lado de um intervalo do PIT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    pub tsc: u64,
    pub apic_ticks: u32,
}

/// O que o intervalo medido diz sobre os dois contadores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calibration {
    pub tsc_hz: u64,
    pub apic_hz: u64,
}

/// Espera `count` ticks do PIT lendo os contadores nas pontas. `None` se o PIT
/// não terminar em `max_polls` consultas (sem PIT, ou clock parado).
pub fn measure<B: RegisterBus>(
    pit: &Pit<B>,
    count: u16,
    max_polls: u32,
    mut read: impl FnMut() -> Reading,
) -> Option<Calibration> {
    pit.arm(count);
    let start = read();
    let mut polls = 0;
    while !pit.expired() {
        polls += 1;
        if polls > max_polls {
            pit.disarm();
            return None;
        }
    }
    let end = read();
    pit.disarm();
    let tsc_delta = end.tsc.checked_sub(start.tsc)?;
    let apic_delta = end.apic_ticks.checked_sub(start.apic_ticks)?;
    Some(Calibration {
        tsc_hz: per_second(tsc_delta, count)?,
        apic_hz: per_second(u64::from(apic_delta), count)?,
    })
}

/// `delta` ticks em `count` ticks do PIT, convertidos para ticks por segundo.
fn per_second(delta: u64, count: u16) -> Option<u64> {
    if count == 0 || delta == 0 {
        return None;
    }
    let hz = u128::from(delta) * u128::from(PIT_HZ) / u128::from(count);
    u64::try_from(hz).ok()
}

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};

    use super::*;

    #[derive(Default)]
    struct FakeBus {
        writes: RefCell<Vec<(usize, u8)>>,
        port_b: Cell<u8>,
        polls_until_expired: Cell<u32>,
    }

    impl RegisterBus for FakeBus {
        fn read8(&self, offset: usize) -> u8 {
            assert_eq!(offset, PORT_B);
            let left = self.polls_until_expired.get();
            if left == 0 {
                return self.port_b.get() | OUT2;
            }
            self.polls_until_expired.set(left - 1);
            self.port_b.get()
        }

        fn write8(&self, offset: usize, value: u8) {
            if offset == PORT_B {
                self.port_b.set(value);
            }
            self.writes.borrow_mut().push((offset, value));
        }

        fn read32(&self, _: usize) -> u32 {
            unreachable!()
        }

        fn write32(&self, _: usize, _: u32) {
            unreachable!()
        }
    }

    #[test]
    fn arm_opens_the_gate_then_programs_mode_0_and_the_count() {
        let pit = Pit::new(FakeBus::default());
        pit.bus.port_b.set(SPEAKER_DATA | 0x0C);
        pit.bus.polls_until_expired.set(10);
        pit.arm(0x2E9C);
        assert_eq!(
            *pit.bus.writes.borrow(),
            [
                (PORT_B, 0x0C | GATE),
                (COMMAND, 0xB0),
                (CHANNEL2_DATA, 0x9C),
                (CHANNEL2_DATA, 0x2E),
            ]
        );
    }

    #[test]
    fn measure_converts_deltas_to_hz() {
        let bus = FakeBus::default();
        bus.polls_until_expired.set(3);
        let pit = Pit::new(bus);
        let mut readings = [
            Reading {
                tsc: 1_000,
                apic_ticks: 0,
            },
            Reading {
                tsc: 1_000 + 29_830_000,
                apic_ticks: 625_000,
            },
        ]
        .into_iter();
        let result = measure(&pit, 11_932, 100, || readings.next().unwrap()).unwrap();
        assert_eq!(result.tsc_hz, 29_830_000 * PIT_HZ / 11_932);
        assert_eq!(result.apic_hz, 625_000 * PIT_HZ / 11_932);
        assert_eq!(pit.bus.port_b.get() & GATE, 0);
    }

    #[test]
    fn measure_gives_up_when_the_pit_never_expires() {
        let bus = FakeBus::default();
        bus.polls_until_expired.set(u32::MAX);
        let pit = Pit::new(bus);
        let result = measure(&pit, 100, 10, || Reading {
            tsc: 0,
            apic_ticks: 0,
        });
        assert_eq!(result, None);
    }

    #[test]
    fn measure_rejects_a_counter_that_went_backwards() {
        let pit = Pit::new(FakeBus::default());
        let mut readings = [
            Reading {
                tsc: 500,
                apic_ticks: 10,
            },
            Reading {
                tsc: 100,
                apic_ticks: 20,
            },
        ]
        .into_iter();
        assert_eq!(measure(&pit, 100, 10, || readings.next().unwrap()), None);
    }

    #[test]
    fn zero_count_or_delta_is_not_a_measurement() {
        assert_eq!(per_second(0, 100), None);
        assert_eq!(per_second(100, 0), None);
    }
}
