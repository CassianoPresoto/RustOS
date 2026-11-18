//! Núcleos secundários: o Limine os acorda e os estaciona (pedido MP); aqui cada um ganha
//! pilha, tabelas do kernel, vetores, controlador de interrupção e timer, e espera o
//! escalonador ligar (SDD 04 e SDD 15).

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use rustos_boot_limine::MpInfo;
use rustos_hal::{BootInfo, Cpu, MAX_CPUS, PhysAddr, VirtAddr};
use rustos_sync::SpinLock;

use crate::memory::Memory;
use crate::stack::Stack;
use crate::{panic, percpu, platform, task};

/// Quanto o BSP espera um núcleo subir: o TCG sem MTTCG é lento.
const START_TIMEOUT_NS: u64 = 3_000_000_000;

/// O que o BSP prepara para um núcleo antes de soltá-lo.
struct Plan {
    resources: platform::SecondaryResources,
    stack: Stack,
}

static PLANS: [SpinLock<Option<Plan>>; MAX_CPUS] = [const { SpinLock::new(None) }; MAX_CPUS];
static STACK_TOPS: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];
static KERNEL_ROOT: AtomicU64 = AtomicU64::new(0);
/// O escalonador da CPU de boot está pronto: os núcleos online entram nele.
static GO: AtomicBool = AtomicBool::new(false);
/// Núcleos que falharam no próprio init, um bit por CPU.
static FAILED: AtomicU32 = AtomicU32::new(0);

/// O que `start` achou.
#[derive(Clone, Copy, Debug)]
pub struct Started {
    /// Núcleos online, o de boot incluído.
    pub online: usize,
    /// Núcleos que o bootloader entregou, o de boot incluído.
    pub found: usize,
    /// Os que o bootloader entregou e o kernel deixou parados: passam do limite `MAX_CPUS`.
    pub skipped: usize,
    /// Máscara dos que subiram mas falharam, ou nunca responderam.
    pub failed: u32,
    /// Os que nunca responderam: ainda podem estar na memória do bootloader.
    pub silent: u32,
}

/// Solta cada núcleo secundário e espera todos ficarem online. Depois de `interrupts::init` e
/// antes de devolver a memória do bootloader: os núcleos ainda andam na pilha e nas tabelas dele.
pub fn start(boot: &impl BootInfo, memory: &mut Memory) -> Started {
    percpu::set_hw_id(0, platform::local_hw_id());
    KERNEL_ROOT.store(platform::kernel_root().as_u64(), Ordering::Release);
    let cpus: Vec<_> = boot.cpus().collect();
    // SAFETY: só o boot mexe nas tabelas agora, e o mapa direto cobre todo frame novo.
    let mut mapper = unsafe { platform::active_space(boot.direct_map_offset()) };
    let (mut next, mut released, mut skipped) = (1, 0_u32, 0_usize);
    for cpu in cpus.iter().filter(|cpu| !cpu.is_boot) {
        if next >= MAX_CPUS {
            skipped += 1;
            continue;
        }
        let index = next;
        next += 1;
        let prepared = memory
            .map_stack_painted(&mut mapper, task::STACK_PAGES)
            .map_err(platform::PlatformError::Map)
            .and_then(|stack| {
                platform::prepare_secondary(memory, &mut mapper).map(|resources| (stack, resources))
            });
        let (stack, resources) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                kprintln!("[rustos] cpu {index}: not prepared: {error:?}");
                FAILED.fetch_or(1 << index, Ordering::Relaxed);
                continue;
            }
        };
        *PLANS[index].lock() = Some(Plan { resources, stack });
        STACK_TOPS[index].store(stack.top().as_u64(), Ordering::Release);
        percpu::set_hw_id(index, platform::ipi_target(cpu.hw_id));
        // SAFETY: o núcleo não é o de boot, é solto uma vez, e a memória do bootloader segue intacta.
        unsafe { rustos_boot_limine::start_cpu(cpu.token, index as u64, ap_entry) };
        released |= 1 << index;
    }
    let silent = wait_online(released);
    Started {
        silent,
        online: percpu::online_count(),
        found: cpus.len().max(1),
        skipped,
        failed: FAILED.load(Ordering::Relaxed),
    }
}

/// Voltas de espera quando não há relógio (`now_ns` devolve `None`) ou ele parou: sem esta
/// reserva o prazo nunca venceria.
const START_SPIN_LIMIT: u64 = 4_000_000_000;

/// Espera os núcleos de `released` (um bit por CPU) ficarem online ou falharem. Devolve os que
/// **não responderam** no prazo: seguem rodando na pilha e nas tabelas do Limine, e a memória
/// dele não pode ser devolvida.
fn wait_online(released: u32) -> u32 {
    let deadline = platform::now_ns().map(|now| now + START_TIMEOUT_NS);
    let pending = || released & !percpu::online_mask() & !FAILED.load(Ordering::Relaxed);
    let mut spins = 0_u64;
    while pending() != 0 {
        spins += 1;
        let expired = match (deadline, platform::now_ns()) {
            (Some(limit), Some(now)) => now > limit,
            _ => false,
        };
        if expired || spins > START_SPIN_LIMIT {
            let silent = pending();
            FAILED.fetch_or(silent, Ordering::Relaxed);
            return silent;
        }
        core::hint::spin_loop();
    }
    0
}

/// O escalonador da CPU de boot está de pé: os núcleos online entram nele e esta espera todos.
pub fn release() {
    GO.store(true, Ordering::Release);
    let online = percpu::online_count();
    let deadline = platform::now_ns().map(|now| now + START_TIMEOUT_NS);
    let mut spins = 0_u64;
    while (task::active_cpus().count_ones() as usize) < online && spins < START_SPIN_LIMIT {
        spins += 1;
        if let (Some(limit), Some(now)) = (deadline, platform::now_ns())
            && now >= limit
        {
            break;
        }
        core::hint::spin_loop();
    }
}

/// Entrada de cada núcleo secundário, na pilha do Limine e com as tabelas dele.
///
/// # Safety
/// Só o Limine chama, uma vez por núcleo, com o `MpInfo` dele que `start_cpu` preparou.
unsafe extern "C" fn ap_entry(info: *const MpInfo) -> ! {
    // SAFETY: o Limine entrega o `MpInfo` deste núcleo.
    let info = unsafe { &*info };
    let index = usize::try_from(info.argument()).unwrap_or(0);
    percpu::install(index);
    platform::secondary_early(PhysAddr::new(KERNEL_ROOT.load(Ordering::Acquire)));
    let top = VirtAddr::new(STACK_TOPS[index].load(Ordering::Acquire));
    // SAFETY: `top` é o fim de uma pilha recém-mapeada e só deste núcleo; `ap_main` não volta.
    unsafe { platform::call_on_stack(top, ap_main, core::ptr::without_provenance_mut(index)) }
}

/// Já na pilha própria: vetores, controlador e timer; avisa que está online e espera o escalonador.
///
/// # Safety
/// Só `ap_entry` chama, com o índice da CPU em `arg`.
unsafe extern "C" fn ap_main(arg: *mut ()) -> ! {
    let index = arg.addr();
    let plan = PLANS[index].lock().take();
    let stack = plan.as_ref().map(|plan| plan.stack);
    let ready = plan.is_some_and(|plan| platform::secondary_init(plan.resources).is_ok());
    let Some(stack) = stack.filter(|_| ready) else {
        FAILED.fetch_or(1 << index, Ordering::Relaxed);
        platform::cpu().halt_forever();
    };
    crate::mitigation::note_cpu();
    percpu::mark_online(index);
    while !GO.load(Ordering::Acquire) {
        panic::stop_if_requested();
        core::hint::spin_loop();
    }
    task::run_secondary(stack)
}
