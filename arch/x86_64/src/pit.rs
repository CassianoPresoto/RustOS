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
    /// Incerteza da medida em partes por milhão: metade do período de uma consulta ao PIT, que
    /// é quanto a transição pode ter escapado das leituras, sobre o intervalo medido.
    pub error_ppm: u64,
}

/// Espera `count` ticks do PIT lendo os contadores a cada consulta. `None` se o PIT não terminar
/// em `max_polls` consultas (sem PIT, ou clock parado). O fim é a leitura da consulta que viu a
/// transição, recuada meia consulta: a transição caiu entre ela e a anterior, e o erro fica em
/// `±período/2` em vez de `+período` (a latência da porta, de uns 14 µs no TCG).
pub fn measure<B: RegisterBus>(
    pit: &Pit<B>,
    count: u16,
    max_polls: u32,
    mut read: impl FnMut() -> Reading,
) -> Option<Calibration> {
    pit.arm(count);
    let start = read();
    let mut previous = start;
    let mut polls = 0;
    while !pit.expired() {
        polls += 1;
        if polls > max_polls {
            pit.disarm();
            return None;
        }
        previous = read();
    }
    let seen = read();
    pit.disarm();
    let tsc_period = seen.tsc.checked_sub(previous.tsc)?;
    let apic_period = seen.apic_ticks.checked_sub(previous.apic_ticks)?;
    let tsc_delta = (seen.tsc - tsc_period / 2).checked_sub(start.tsc)?;
    let apic_delta = (seen.apic_ticks - apic_period / 2).checked_sub(start.apic_ticks)?;
    let error_ppm = if tsc_delta == 0 {
        u64::MAX
    } else {
        u64::try_from(u128::from(tsc_period / 2) * 1_000_000 / u128::from(tsc_delta)).ok()?
    };
    Some(Calibration {
        tsc_hz: per_second(tsc_delta, count)?,
        apic_hz: per_second(u64::from(apic_delta), count)?,
        error_ppm,
    })
}

/// Mede com `min_count` e, se a incerteza passar de `max_ppm`, mede de novo com a janela que a
/// reduziria a esse valor, até `max_count` (o erro cai com o inverso da janela). Hardware de
/// leitura rápida fecha na primeira; o TCG, de porta lenta, estende. Devolve a de menor incerteza.
pub fn calibrate<B: RegisterBus>(
    pit: &Pit<B>,
    min_count: u16,
    max_count: u16,
    max_ppm: u64,
    max_polls: u32,
    mut read: impl FnMut() -> Reading,
) -> Option<Calibration> {
    let first = measure(pit, min_count, max_polls, &mut read)?;
    if first.error_ppm <= max_ppm || min_count >= max_count {
        return Some(first);
    }
    let wanted = u128::from(min_count) * u128::from(first.error_ppm) / u128::from(max_ppm.max(1));
    let count = u16::try_from(wanted + 1).unwrap_or(u16::MAX).min(max_count);
    let second = measure(pit, count.max(min_count), max_polls, &mut read)?;
    Some(if second.error_ppm < first.error_ppm {
        second
    } else {
        first
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
        /// Se não for zero, cada `arm` reabastece as consultas: uma por `polls_per_1000_ticks`/1000 ticks.
        polls_per_1000_ticks: Cell<u32>,
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
                self.port_b.set(value & !OUT2);
            }
            self.writes.borrow_mut().push((offset, value));
            let per_tick = self.polls_per_1000_ticks.get();
            if offset == CHANNEL2_DATA && per_tick > 0 {
                let writes = self.writes.borrow();
                if let [.., (_, low), (_, high)] = writes[..] {
                    let count = u32::from(u16::from_le_bytes([low, high]));
                    self.polls_until_expired.set(count * per_tick / 1000);
                }
            }
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

    /// Leituras que andam `tsc_step` ciclos e `apic_step` ticks por chamada.
    fn stepping(tsc_step: u64, apic_step: u32) -> impl FnMut() -> Reading {
        let mut calls = 0_u64;
        move || {
            calls += 1;
            Reading {
                tsc: 1_000 + calls * tsc_step,
                apic_ticks: u32::try_from(calls).unwrap() * apic_step,
            }
        }
    }

    #[test]
    fn measure_converts_deltas_to_hz_and_centers_the_end_on_the_poll() {
        let bus = FakeBus::default();
        bus.polls_until_expired.set(4);
        let pit = Pit::new(bus);
        // 5 leituras: início, 3 consultas sem fim, a que viu o fim. Período de 1 000 ciclos:
        // o fim recua 500 do último, e o intervalo medido é 4 000 - 500.
        let result = measure(&pit, 11_932, 100, stepping(1_000, 100)).unwrap();
        assert_eq!(result.tsc_hz, 3_500 * PIT_HZ / 11_932);
        assert_eq!(result.apic_hz, 350 * PIT_HZ / 11_932);
        assert_eq!(result.error_ppm, 500 * 1_000_000 / 3_500);
        assert_eq!(pit.bus.port_b.get() & GATE, 0);
    }

    #[test]
    fn measure_gives_up_when_the_pit_never_expires() {
        let bus = FakeBus::default();
        bus.polls_until_expired.set(u32::MAX);
        let pit = Pit::new(bus);
        assert_eq!(measure(&pit, 100, 10, stepping(100, 100)), None);
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
    fn calibrate_stops_at_the_first_window_when_the_reading_is_fast() {
        let bus = FakeBus::default();
        bus.polls_per_1000_ticks.set(500);
        let pit = Pit::new(bus);
        // 2 983 consultas de 1 ciclo: incerteza de 0,5 ciclo em 2 983, cerca de 170 ppm.
        let result = calibrate(&pit, 5_966, 23_864, 500, 100_000, stepping(100, 100)).unwrap();
        assert!(result.error_ppm <= 500, "{}", result.error_ppm);
        let writes = pit.bus.writes.borrow();
        assert_eq!(
            writes
                .iter()
                .filter(|(offset, _)| *offset == COMMAND)
                .count(),
            1
        );
    }

    #[test]
    fn calibrate_asks_for_a_longer_window_when_the_reading_is_slow() {
        let bus = FakeBus::default();
        bus.polls_per_1000_ticks.set(10);
        let pit = Pit::new(bus);
        // 59 consultas na primeira janela: incerteza de ~8 000 ppm, acima do teto de 500; a
        // segunda janela vai ao máximo (23 864 ticks, 0x5D38) e a de menor incerteza vence.
        let result = calibrate(&pit, 5_966, 23_864, 500, 100_000, stepping(100, 100)).unwrap();
        let writes = pit.bus.writes.borrow();
        assert_eq!(
            writes
                .iter()
                .filter(|(offset, _)| *offset == COMMAND)
                .count(),
            2
        );
        assert!(writes.contains(&(CHANNEL2_DATA, 0x38)) && writes.contains(&(CHANNEL2_DATA, 0x5D)));
        assert!(result.error_ppm < 4_000, "{}", result.error_ppm);
    }

    #[test]
    fn zero_count_or_delta_is_not_a_measurement() {
        assert_eq!(per_second(0, 100), None);
        assert_eq!(per_second(100, 0), None);
    }
}
