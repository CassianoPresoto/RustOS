//! Estado por CPU: um bloco `PerCpu` para cada núcleo, achado pelo `GS` (x86_64) ou pelo
//! `TPIDR_EL1` (aarch64). Estado por CPU vem antes de lock global (guidelines 05).

use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};

use rustos_hal::{Cpu, CpuLocal, MAX_CPUS};

use crate::sync::IrqLock;
use crate::task::Queue;
use crate::{frames, platform};

/// O que só a CPU dona escreve e as outras só leem, mais a fila dela.
/// O primeiro campo é o endereço do próprio bloco: é o que `gs:[0]` lê.
#[derive(Debug)]
#[repr(C, align(64))]
pub struct PerCpu {
    block: AtomicUsize,
    index: AtomicU32,
    /// LAPIC ID ou afinidade do `MPIDR`: o destino de um IPI.
    hw_id: AtomicU64,
    in_irq: AtomicBool,
    locks_held: AtomicU32,
    /// O prazo que está no timer de um tiro desta CPU agora.
    armed_ns: AtomicU64,
    /// Tarefas prontas e correndo na fila desta CPU, publicado sob o lock da fila e lido
    /// sem ele pelas outras CPUs para escolher onde colocar e de onde puxar.
    pub load: AtomicU32,
    /// Slot da tarefa que ocupa a CPU (a ociosa quando a fila está vazia).
    pub current: AtomicU32,
    /// Slot da tarefa que acabou de sair desta CPU: `on_cpu` dela só baixa depois da troca.
    pub prev: AtomicU32,
    /// Já há um IPI de reescalonar a caminho desta CPU.
    pub kick_pending: AtomicBool,
    /// A CPU está na ociosa, dormindo ou prestes a dormir.
    pub idle: AtomicBool,
    pub next_balance_ns: AtomicU64,
    /// A fila de execução desta CPU. Ver o SDD 15 para a regra de lock.
    pub queue: IrqLock<Queue>,
    /// Frames soltos desta CPU, na frente do buddy.
    pub frames: IrqLock<frames::Cache>,
    pub stats: Stats,
}

/// Contadores de uma CPU, de leitura livre por qualquer outra.
#[derive(Debug, Default)]
pub struct Stats {
    pub irqs: AtomicU64,
    pub timer_irqs: AtomicU64,
    pub ipis_sent: AtomicU64,
    pub ipis_received: AtomicU64,
    pub switches: AtomicU64,
    pub pulled: AtomicU64,
}

impl Stats {
    const fn new() -> Self {
        Self {
            irqs: AtomicU64::new(0),
            timer_irqs: AtomicU64::new(0),
            ipis_sent: AtomicU64::new(0),
            ipis_received: AtomicU64::new(0),
            switches: AtomicU64::new(0),
            pulled: AtomicU64::new(0),
        }
    }
}

impl PerCpu {
    const fn new() -> Self {
        Self {
            block: AtomicUsize::new(0),
            index: AtomicU32::new(0),
            hw_id: AtomicU64::new(0),
            in_irq: AtomicBool::new(false),
            locks_held: AtomicU32::new(0),
            armed_ns: AtomicU64::new(u64::MAX),
            load: AtomicU32::new(0),
            current: AtomicU32::new(0),
            prev: AtomicU32::new(NO_TASK),
            kick_pending: AtomicBool::new(false),
            idle: AtomicBool::new(false),
            next_balance_ns: AtomicU64::new(0),
            queue: IrqLock::new(Queue::new()),
            frames: IrqLock::new(frames::Cache::new()),
            stats: Stats::new(),
        }
    }

    pub fn index(&self) -> usize {
        self.index.load(Ordering::Relaxed) as usize
    }

    pub fn hw_id(&self) -> u64 {
        self.hw_id.load(Ordering::Relaxed)
    }

    pub fn in_irq(&self) -> bool {
        self.in_irq.load(Ordering::Relaxed)
    }

    pub fn set_in_irq(&self, value: bool) {
        self.in_irq.store(value, Ordering::Relaxed);
    }

    pub fn locks_held(&self) -> u32 {
        self.locks_held.load(Ordering::Relaxed)
    }

    /// Só esta CPU escreve o contador: uma leitura e uma escrita bastam.
    pub fn lock_taken(&self) {
        let held = self.locks_held.load(Ordering::Relaxed);
        self.locks_held.store(held + 1, Ordering::Relaxed);
    }

    pub fn lock_released(&self) {
        let held = self.locks_held.load(Ordering::Relaxed);
        self.locks_held.store(held - 1, Ordering::Relaxed);
    }

    pub fn armed_ns(&self) -> u64 {
        self.armed_ns.load(Ordering::Relaxed)
    }

    pub fn set_armed_ns(&self, value: u64) {
        self.armed_ns.store(value, Ordering::Relaxed);
    }
}

/// `prev` sem ninguém para liberar.
pub const NO_TASK: u32 = u32::MAX;

static CPUS: [PerCpu; MAX_CPUS] = [const { PerCpu::new() }; MAX_CPUS];
/// Máscara das CPUs que terminaram o próprio init, um bit por CPU.
static ONLINE_MASK: AtomicU32 = AtomicU32::new(0);

/// Registra o bloco da CPU `index` nesta CPU. Antes do primeiro `IrqLock` dela.
pub fn install(index: usize) {
    let cpu = &CPUS[index];
    cpu.index
        .store(u32::try_from(index).unwrap_or(0), Ordering::Relaxed);
    let block = ptr::from_ref(cpu).expose_provenance();
    cpu.block.store(block, Ordering::Relaxed);
    platform::cpu_local().install(block);
}

/// Grava o identificador de hardware de uma CPU que ainda não rodou (o BSP o faz por ela).
pub fn set_hw_id(index: usize, hw_id: u64) {
    CPUS[index].hw_id.store(hw_id, Ordering::Relaxed);
}

/// O bloco da CPU que executa isto. Só depois de `install` nela: antes, o bloco é zero.
/// Com a IRQ ligada, a tarefa pode migrar entre ler o bloco e ler um campo dele: quem lê
/// campo da CPU "atual" fora de uma região mascarada usa `pinned`.
pub fn current() -> &'static PerCpu {
    let block = platform::cpu_local().block();
    // SAFETY: `install` guardou o endereço de um elemento de `CPUS`, que vive para sempre.
    unsafe { &*ptr::with_exposed_provenance::<PerCpu>(block) }
}

/// Roda `read` com a interrupção mascarada: a tarefa não migra nem é preemptada entre achar
/// o bloco da CPU e ler dele.
pub fn pinned<R>(read: impl FnOnce(&'static PerCpu) -> R) -> R {
    let cpu = platform::cpu();
    let were_enabled = cpu.save_and_disable_interrupts();
    let value = read(current());
    cpu.restore_interrupts(were_enabled);
    value
}

/// O bloco da CPU `index`, de qualquer CPU.
pub fn get(index: usize) -> &'static PerCpu {
    &CPUS[index]
}

/// Marca a CPU como online.
pub fn mark_online(index: usize) {
    ONLINE_MASK.fetch_or(1 << index, Ordering::Release);
}

/// A CPU de boot conta como online desde o início.
pub fn mark_boot_online() {
    ONLINE_MASK.fetch_or(1, Ordering::Release);
}

pub fn online_count() -> usize {
    online_mask().count_ones() as usize
}

pub fn online_mask() -> u32 {
    ONLINE_MASK.load(Ordering::Acquire)
}
