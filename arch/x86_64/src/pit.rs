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
/// Trava a contagem atual do canal 2 para leitura (comando de latch: canal 2, acesso 0).
const CHANNEL2_LATCH: u8 = 0x80;
const GATE: u8 = 1 << 0;
const SPEAKER_DATA: u8 = 1 << 1;

#[derive(Debug)]
pub struct Pit<B> {
    bus: B,
}

impl<B: RegisterBus> Pit<B> {
    pub const fn new(bus: B) -> Self {
        Self { bus }
    }

    /// Dispara o canal 2 em modo 0 a partir de 0xFFFF (54,9 ms), sem som.
    pub fn start(&self) {
        let port_b = (self.bus.read8(PORT_B) & !SPEAKER_DATA) | GATE;
        self.bus.write8(PORT_B, port_b);
        self.bus.write8(COMMAND, CHANNEL2_MODE0);
        self.bus.write8(CHANNEL2_DATA, 0xFF);
        self.bus.write8(CHANNEL2_DATA, 0xFF);
    }

    /// Trava a contagem; o instante da amostra é o do comando de latch.
    fn latch(&self) {
        self.bus.write8(COMMAND, CHANNEL2_LATCH);
    }

    /// Lê a contagem travada (baixo, depois alto).
    fn read_latched(&self) -> u16 {
        let low = self.bus.read8(CHANNEL2_DATA);
        let high = self.bus.read8(CHANNEL2_DATA);
        u16::from_le_bytes([low, high])
    }

    /// Fecha o gate do canal 2.
    pub fn stop(&self) {
        self.bus.write8(PORT_B, self.bus.read8(PORT_B) & !GATE);
    }
}

/// Duas leituras dos contadores, uma de cada lado de um instante.
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
    /// Incerteza máxima da medida, em partes por milhão: a largura das duas amostras das pontas
    /// (o TSC lido de cada lado do comando de latch) sobre o intervalo.
    pub error_ppm: u64,
    /// Ticks do PIT que a janela durou.
    pub ticks: u32,
}

/// Quando parar e quando recusar (SDD 10).
#[derive(Clone, Copy, Debug)]
pub struct Policy {
    /// Janela em que se aceita a incerteza que houver, se a extrapolação mostra que mais tempo não
    /// chegaria ao alvo (ticks do PIT).
    pub min_ticks: u32,
    /// Teto da janela (ticks do PIT).
    pub max_ticks: u32,
    /// Incerteza em que a medida fecha na hora.
    pub target_ppm: u64,
    /// Acima disto a medida é recusada.
    pub refuse_ppm: u64,
    /// Tempo máximo, em ciclos de TSC, para o PIT andar: sem ele, o PIT está parado ou ausente.
    pub max_cycles: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// O contador do PIT não andou dentro de `max_cycles`.
    Stuck,
    /// A janela chegou ao teto e a incerteza segue acima de `refuse_ppm`.
    TooUncertain(u64),
}

#[derive(Clone, Copy)]
struct Sample {
    count: u16,
    before: Reading,
    after: Reading,
}

impl Sample {
    fn take<B: RegisterBus>(pit: &Pit<B>, read: &mut impl FnMut() -> Reading) -> Self {
        let before = read();
        pit.latch();
        let after = read();
        Self {
            count: pit.read_latched(),
            before,
            after,
        }
    }

    /// Largura da amostra em ciclos de TSC: onde, entre as duas leituras, o latch caiu.
    fn width(&self) -> u64 {
        self.after.tsc.saturating_sub(self.before.tsc)
    }

    fn tsc(&self) -> u128 {
        u128::midpoint(u128::from(self.before.tsc), u128::from(self.after.tsc))
    }

    fn apic(&self) -> u128 {
        u128::midpoint(
            u128::from(self.before.apic_ticks),
            u128::from(self.after.apic_ticks),
        )
    }
}

/// Número de amostras iniciais, de que se guarda a mais estreita como origem.
const ORIGINS: usize = 3;

/// Liga o PIT e amostra o contador dele (com latch) junto do TSC e do contador do APIC, numa janela
/// única que cresce até a incerteza fechar (`Policy`, SDD 10). Sempre fecha o gate.
pub fn calibrate<B: RegisterBus>(
    pit: &Pit<B>,
    policy: Policy,
    mut read: impl FnMut() -> Reading,
) -> Result<Calibration, Failure> {
    pit.start();
    let result = run(pit, policy, &mut read);
    pit.stop();
    result
}

fn run<B: RegisterBus>(
    pit: &Pit<B>,
    policy: Policy,
    read: &mut impl FnMut() -> Reading,
) -> Result<Calibration, Failure> {
    let mut origin = Sample::take(pit, read);
    for _ in 1..ORIGINS {
        let next = Sample::take(pit, read);
        if next.width() < origin.width() {
            origin = next;
        }
    }
    loop {
        let sample = Sample::take(pit, read);
        let ticks = u32::from(origin.count.wrapping_sub(sample.count));
        let delta = sample.tsc().saturating_sub(origin.tsc());
        if ticks == 0 || delta == 0 {
            let spent = sample.after.tsc.saturating_sub(origin.before.tsc);
            if spent > policy.max_cycles {
                return Err(Failure::Stuck);
            }
            continue;
        }
        let error = u128::from(origin.width() + sample.width()) / 2;
        let ppm = u64::try_from(error * 1_000_000 / delta).unwrap_or(u64::MAX);
        let projected = u128::from(ppm) * u128::from(ticks) / u128::from(policy.max_ticks.max(1));
        let capped = ticks >= policy.max_ticks;
        let hopeless = ticks >= policy.min_ticks && projected > u128::from(policy.target_ppm);
        let overdue = sample.after.tsc.saturating_sub(origin.before.tsc) > policy.max_cycles;
        if ppm <= policy.target_ppm || hopeless || capped || overdue {
            if ppm > policy.refuse_ppm {
                if capped || overdue {
                    return Err(Failure::TooUncertain(ppm));
                }
                continue;
            }
            let apic = sample.apic().saturating_sub(origin.apic());
            return Ok(Calibration {
                tsc_hz: per_second(delta, ticks)?,
                apic_hz: per_second(apic, ticks)?,
                error_ppm: ppm,
                ticks,
            });
        }
    }
}

/// `delta` ticks em `ticks` ticks do PIT, convertidos para ticks por segundo.
fn per_second(delta: u128, ticks: u32) -> Result<u64, Failure> {
    if ticks == 0 || delta == 0 {
        return Err(Failure::Stuck);
    }
    let hz = delta * u128::from(PIT_HZ) / u128::from(ticks);
    u64::try_from(hz).map_err(|_| Failure::TooUncertain(u64::MAX))
}

#[cfg(test)]
mod tests {
    use core::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    /// Um PIT que gasta `ns_per_latch` de tempo entre dois comandos de latch (a 1 GHz, ciclos
    /// iguais a ns) e devolve a contagem travada.
    #[derive(Default)]
    struct FakeBus {
        writes: RefCell<Vec<(usize, u8)>>,
        port_b: Cell<u8>,
        /// Tempo desde o início da contagem, em ns vezes `PIT_HZ`: dividido por 1e9 dá os ticks.
        elapsed: Cell<u64>,
        latched: Cell<u16>,
        ns_per_latch: Cell<u64>,
        /// Atraso que o relógio sofreu depois do último latch e que o PIT também viveu.
        pending_ns: Rc<Cell<u64>>,
        read_high: Cell<bool>,
    }

    impl RegisterBus for FakeBus {
        fn read8(&self, offset: usize) -> u8 {
            match offset {
                PORT_B => self.port_b.get(),
                CHANNEL2_DATA => {
                    let [low, high] = self.latched.get().to_le_bytes();
                    let high_turn = self.read_high.replace(!self.read_high.get());
                    if high_turn { high } else { low }
                }
                _ => unreachable!(),
            }
        }

        fn write8(&self, offset: usize, value: u8) {
            self.writes.borrow_mut().push((offset, value));
            match (offset, value) {
                (PORT_B, _) => self.port_b.set(value),
                (COMMAND, CHANNEL2_LATCH) => {
                    let ticks = u16::try_from(self.elapsed.get() / 1_000_000_000).unwrap();
                    self.latched.set(0xFFFF_u16.wrapping_sub(ticks));
                    let ns = self.ns_per_latch.get() + self.pending_ns.replace(0);
                    self.elapsed.set(self.elapsed.get() + ns * PIT_HZ);
                    self.read_high.set(false);
                }
                (CHANNEL2_DATA, 0xFF) => self.elapsed.set(0),
                _ => {}
            }
        }

        fn read32(&self, _: usize) -> u32 {
            unreachable!()
        }

        fn write32(&self, _: usize, _: u32) {
            unreachable!()
        }
    }

    const POLICY: Policy = Policy {
        min_ticks: 5_966,
        max_ticks: 23_864,
        target_ppm: 400,
        refuse_ppm: 10_000,
        max_cycles: 300_000_000,
    };

    /// Um relógio de 1 GHz que gasta `call_ns` por leitura; `spike` é a chamada (de 1) que gasta
    /// `spike_ns` a mais. O PIT do `bus` anda o tempo de duas leituras por amostra.
    fn clock(
        call_ns: u64,
        spike: Option<(u64, u64)>,
        pending: &Rc<Cell<u64>>,
    ) -> impl FnMut() -> Reading {
        let pending = Rc::clone(pending);
        let (mut calls, mut tsc) = (0_u64, 5_000_u64);
        move || {
            calls += 1;
            tsc += call_ns;
            if let Some((at, extra)) = spike
                && calls == at
            {
                tsc += extra;
                pending.set(extra);
            }
            Reading {
                tsc,
                apic_ticks: u32::try_from(tsc / 16).unwrap(),
            }
        }
    }

    /// Duas leituras de `call_ns` separam dois latches.
    fn bus_for(call_ns: u64) -> Pit<FakeBus> {
        let bus = FakeBus::default();
        bus.ns_per_latch.set(2 * call_ns);
        Pit::new(bus)
    }

    fn within(hz: u64, expected: u64, ppm: u64) -> bool {
        hz.abs_diff(expected) * 1_000_000 <= expected * ppm
    }

    #[test]
    fn start_programs_channel_2_from_ffff_and_stop_closes_the_gate() {
        let pit = Pit::new(FakeBus::default());
        pit.bus.port_b.set(SPEAKER_DATA | 0x0C);
        pit.start();
        assert_eq!(
            *pit.bus.writes.borrow(),
            [
                (PORT_B, 0x0C | GATE),
                (COMMAND, 0xB0),
                (CHANNEL2_DATA, 0xFF),
                (CHANNEL2_DATA, 0xFF),
            ]
        );
        pit.stop();
        assert_eq!(pit.bus.port_b.get() & GATE, 0);
    }

    #[test]
    fn a_fast_reader_closes_at_the_target_before_the_minimum_window() {
        let pit = bus_for(500);
        let result = calibrate(&pit, POLICY, clock(500, None, &pit.bus.pending_ns)).unwrap();
        assert!(result.error_ppm <= 400, "{result:?}");
        assert!(result.ticks < POLICY.min_ticks, "{result:?}");
        assert!(within(result.tsc_hz, 1_000_000_000, 1_000), "{result:?}");
        assert_eq!(pit.bus.port_b.get() & GATE, 0);
    }

    #[test]
    fn a_slow_reader_gives_up_extending_at_the_minimum_window() {
        // 14 us por leitura (o inb do TCG): a incerteza a 5 ms é de uns 2 800 ppm e, a 20 ms,
        // ainda seria de 700: não chega a 400, então fecha com a janela mínima.
        let pit = bus_for(14_000);
        let result = calibrate(&pit, POLICY, clock(14_000, None, &pit.bus.pending_ns)).unwrap();
        assert!(result.ticks >= POLICY.min_ticks, "{result:?}");
        assert!(result.ticks < POLICY.min_ticks + 100, "{result:?}");
        assert!(
            result.error_ppm > 400 && result.error_ppm < 10_000,
            "{result:?}"
        );
        assert!(within(result.tsc_hz, 1_000_000_000, 5_000), "{result:?}");
    }

    #[test]
    fn a_latency_spike_in_the_last_pair_does_not_spoil_the_result() {
        let pit = bus_for(2_000);
        // Uma leitura que demora 2 ms a mais, perto de onde o intervalo fecharia: a amostra
        // larga empurra a incerteza para cima e a medida continua até a próxima, estreita.
        let spiked = calibrate(
            &pit,
            POLICY,
            clock(2_000, Some((900, 2_000_000)), &pit.bus.pending_ns),
        )
        .unwrap();
        let quiet = bus_for(2_000);
        let calm = calibrate(&quiet, POLICY, clock(2_000, None, &quiet.bus.pending_ns)).unwrap();
        assert!(within(spiked.tsc_hz, 1_000_000_000, 5_000), "{spiked:?}");
        assert!(spiked.ticks >= calm.ticks, "{spiked:?} {calm:?}");
    }

    #[test]
    fn a_reader_too_slow_for_one_percent_is_refused_at_the_cap() {
        // 300 us por leitura: 15 000 ppm no teto de 20 ms.
        let pit = bus_for(300_000);
        let result = calibrate(&pit, POLICY, clock(300_000, None, &pit.bus.pending_ns));
        assert!(
            matches!(result, Err(Failure::TooUncertain(ppm)) if ppm > 10_000),
            "{result:?}"
        );
        assert_eq!(pit.bus.port_b.get() & GATE, 0);
    }

    #[test]
    fn a_stuck_pit_is_refused_by_time_not_by_polls() {
        let bus = FakeBus::default();
        bus.ns_per_latch.set(0);
        let pit = Pit::new(bus);
        let result = calibrate(&pit, POLICY, clock(1_000_000, None, &pit.bus.pending_ns));
        assert_eq!(result, Err(Failure::Stuck));
        let writes = pit.bus.writes.borrow();
        let latches = writes
            .iter()
            .filter(|w| **w == (COMMAND, CHANNEL2_LATCH))
            .count();
        // 300 ms a 1 us por leitura de 2 leituras por amostra: algumas centenas de amostras.
        assert!(latches < 1_000, "{latches}");
    }
}
