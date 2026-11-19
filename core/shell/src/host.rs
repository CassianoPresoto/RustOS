//! Porta do shell: o que ele precisa do sistema. Quem implementa é o `kernel/`.

use rustos_hal::{BootTimeline, BootloaderId};

/// Porta do shell: o que ele precisa do sistema. O kernel implementa.
pub trait Host {
    /// Retrato do sistema, lido por `fetch`, `mem`, `uname` e `version`.
    fn info(&self) -> SystemInfo<'_>;
    /// Tempo desde o boot em nanossegundos; `None` enquanto não há relógio.
    fn uptime_ns(&self) -> Option<u64>;
    /// Tamanho do terminal em (colunas, linhas); a serial usa (80, 24).
    fn terminal_size(&self) -> (u16, u16);
    /// Se o terminal tem tela alternativa; a serial, que é de quem está do outro
    /// lado, conta como tendo.
    fn has_alternate_screen(&self) -> bool {
        true
    }
    /// Provoca a exceção pedida para testar o caminho de falha. `PageFault` e
    /// `WriteText` só voltam se a proteção faltar; `false` quando a plataforma
    /// ainda não sabe provocar.
    fn fault_test(&self, _kind: FaultTest) -> bool {
        false
    }
    /// Um número aleatório de 64 bits do gerador do kernel; `None` sem gerador.
    fn random_u64(&self) -> Option<u64> {
        None
    }
    /// Layout do teclado em uso; `None` quando não há teclado.
    fn keymap(&self) -> Option<&'static str> {
        None
    }
    /// Troca o layout pelo nome (`us`, `br`); `false` se o nome é desconhecido.
    fn set_keymap(&self, _name: &str) -> bool {
        false
    }
    /// Chama `visit` uma vez por tarefa viva, em ordem de id; é o que `ps` lista.
    fn tasks(&self, _visit: &mut dyn FnMut(&TaskInfo<'_>)) {}
    /// Chama `visit` uma vez por CPU ativa, em ordem; é o que `ps` e `bench` mostram por CPU.
    fn cpus(&self, _visit: &mut dyn FnMut(&CpuInfo)) {}
    /// Chama `visit` com o nome de cada lock global e quantas tentativas de pegá-lo o acharam
    /// preso, desde o boot: a medida de disputa entre CPUs.
    fn lock_contention(&self, _visit: &mut dyn FnMut(&str, u64)) {}
    /// Roda as medidas de tarefa (troca de contexto e jitter de acordar) e devolve
    /// o resultado; `None` quando o sistema ainda não tem tarefas. Leva centenas de ms.
    fn bench(&self) -> Option<BenchReport> {
        None
    }
    /// As medidas curtas de zerar memória e do gerador aleatório, as que o boot só faz no
    /// modo de teste; `None` quando o sistema não as tem.
    fn micro_bench(&self) -> Option<MicroBench> {
        None
    }
    /// Chama `visit` uma vez por arquivo do initramfs, em ordem; é o que `ls` lista.
    fn programs(&self, _visit: &mut dyn FnMut(&ProgramInfo<'_>)) -> Listing {
        Listing::NoInitramfs
    }
    /// Carrega o programa `name` do initramfs e o roda como processo. Com `wait`, só volta quando
    /// ele termina e diz como; sem `wait`, volta assim que ele começou.
    fn run_program(&self, _name: &str, _wait: bool) -> RunOutcome {
        RunOutcome::NoInitramfs
    }
}

/// Um arquivo do initramfs, como `ls` o mostra.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProgramInfo<'a> {
    pub name: &'a str,
    pub bytes: u64,
}

/// Como terminou a leitura do initramfs por `Host::programs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listing {
    Done,
    /// O boot não trouxe initramfs, ou não conseguiu mapeá-lo.
    NoInitramfs,
    /// O arquivo cpio está malformado.
    Unreadable,
}

/// O que `Host::run_program` conta do programa.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunOutcome {
    /// Terminou por conta própria, com este código de saída.
    Exited {
        pid: u32,
        code: i64,
    },
    /// O kernel o encerrou: o processo fez algo que a CPU recusou.
    Killed {
        pid: u32,
        cause: KillCause,
    },
    /// Começou e segue rodando (`run` sem esperar).
    Started {
        pid: u32,
    },
    /// Nenhum arquivo com esse nome no initramfs.
    NotFound,
    /// O arquivo existe mas não é um executável que o kernel aceite.
    NotExecutable,
    /// Faltou memória para criar o processo.
    NoMemory,
    /// A tabela de tarefas do kernel está cheia.
    TooManyProcesses,
    NoInitramfs,
    /// Qualquer outra falha ao criar o processo.
    Failed,
}

/// Por que o kernel encerrou um processo. Só valores de usuário: nenhum endereço do kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KillCause {
    /// Acesso a endereço de usuário sem mapa ou sem permissão; abaixo de `USER_MIN` é nulo ou quase.
    PageFault {
        address: u64,
        access: Access,
    },
    /// A falta foi na metade do kernel: o endereço não sai do kernel.
    KernelPageFault {
        access: Access,
    },
    IllegalInstruction,
    Unaligned,
    Arithmetic,
    Breakpoint,
    /// A falta de página não se resolveu por falta de frame.
    OutOfMemory,
    Other,
}

/// O tipo do acesso que faltou.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
    Execute,
}

/// Zerar memória e gerar números aleatórios: o que o `bench` mostra além das tarefas.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MicroBench {
    /// Zerar 1 KiB, 4 KiB, 4 KiB numa faixa de 1 MiB e um par alocar-liberar de 4 KiB do heap,
    /// em décimos de ns.
    pub zero_kib_tenths_ns: u64,
    pub zero_page_tenths_ns: u64,
    pub zero_cold_tenths_ns: u64,
    pub zero_pair_tenths_ns: u64,
    /// Vazão do ChaCha20 em MB/s, custo de uma reposição e de um `next_u64` (décimos de ns).
    pub random_mb_per_s: u64,
    pub random_reseed_ns: u64,
    pub random_u64_tenths_ns: u64,
}

/// Em que ponto do ciclo de vida uma tarefa está.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskState {
    Running,
    Ready,
    /// Dormindo até um prazo (`sleep_until`).
    Sleeping,
    /// Esperando um evento (entrada, `park`).
    Blocked,
}

/// Uma CPU ativa, como `ps` e `bench` a mostram.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuInfo {
    pub index: u8,
    /// Tarefas prontas e correndo na fila dela agora.
    pub load: u32,
    /// Tempo que a ociosa dela acumulou, em nanossegundos.
    pub idle_ns: u64,
    /// Trocas de contexto feitas nela.
    pub switches: u64,
    /// IPIs de reescalonar que ela mandou e que recebeu.
    pub ipis_sent: u64,
    pub ipis_received: u64,
    /// Tarefas que ela puxou de outra fila.
    pub pulled: u64,
}

/// Quantos bytes do nome de uma tarefa o `ps` mostra.
pub const TASK_NAME_BYTES: usize = 16;

/// Nome curto e copiável de uma tarefa: o do programa, cortado em `TASK_NAME_BYTES` sem partir
/// um caractere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskName {
    bytes: [u8; TASK_NAME_BYTES],
    len: u8,
}

impl TaskName {
    pub const EMPTY: Self = Self {
        bytes: [0; TASK_NAME_BYTES],
        len: 0,
    };

    pub fn new(name: &str) -> Self {
        let mut end = name.len().min(TASK_NAME_BYTES);
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        let mut bytes = [0; TASK_NAME_BYTES];
        bytes[..end].copy_from_slice(&name.as_bytes()[..end]);
        Self {
            bytes,
            len: u8::try_from(end).unwrap_or(0),
        }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Uma linha do `ps`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskInfo<'a> {
    pub id: u32,
    pub name: &'a str,
    /// O processo de usuário que a tarefa roda; `None` na tarefa só de kernel.
    pub pid: Option<u32>,
    pub state: TaskState,
    /// A CPU dona da tarefa (a fila em que ela está).
    pub cpu: u8,
    /// Tempo de CPU acumulado, em nanossegundos.
    pub cpu_ns: u64,
    /// Quantas vezes o escalonador a escolheu para rodar.
    pub switches: u64,
    /// Marcada como usuária de FPU/SIMD: paga o salvamento do estado na troca.
    pub simd: bool,
    /// Pico de uso da pilha do kernel da tarefa e o tamanho dela, em bytes; zero sem medida.
    pub stack_peak: u32,
    pub stack_bytes: u32,
}

/// Uma linha da medida de escala: o tempo total de `tasks` tarefas de CPU em `cpus` núcleos.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScaleSample {
    pub cpus: u8,
    pub tasks: u8,
    /// Zero quando a máquina não tem tantos núcleos.
    pub total_ns: u64,
}

/// A medida de contenção da memória: todas as CPUs alocando e devolvendo ao mesmo tempo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StormSample {
    pub cpus: u8,
    /// Voltas de cada CPU.
    pub iterations: u32,
    /// Tempo total de frames soltos pelo lock do buddy, e com o cache por CPU, em ns.
    pub locked_ns: u64,
    pub cached_ns: u64,
    /// Disputas pelo lock do buddy nas duas rodadas.
    pub locked_contention: u32,
    pub cached_contention: u32,
    /// O heap (um lock global): tempo total em ns e disputas.
    pub heap_ns: u64,
    pub heap_contention: u32,
}

/// Resultado do `bench`: tudo em nanossegundos.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BenchReport {
    /// Contenção da memória com todas as CPUs alocando.
    pub storm: StormSample,
    /// Custo de uma troca de contexto entre duas tarefas na mesma CPU, sem estado de SIMD.
    pub switch_ns: u64,
    /// O mesmo, com as duas tarefas salvando e restaurando FPU/SIMD.
    pub switch_simd_ns: u64,
    /// O mesmo com as duas tarefas em CPUs diferentes: cada passagem de vez é um acordar
    /// remoto, com IPI. Zero com uma CPU só.
    pub switch_cross_ns: u64,
    /// Uma ida e volta de IPI entre duas CPUs. Zero com uma CPU só.
    pub ipi_rtt_ns: u64,
    /// Tempo total de tarefas de CPU em 1, 2 e 4 núcleos.
    pub scale: [ScaleSample; 3],
    /// CPUs ativas.
    pub cpus: u8,
    /// Quantas vezes se mediu o atraso de `sleep_until`.
    pub wake_samples: u32,
    /// Atraso médio e máximo de acordar, com o resto do sistema parado.
    pub wake_mean_ns: u64,
    pub wake_max_ns: u64,
    /// O mesmo, com uma tarefa ocupando a CPU.
    pub wake_loaded_mean_ns: u64,
    pub wake_loaded_max_ns: u64,
    /// Acordar de uma tarefa, a partir de outra na mesma CPU, uma de fatia curta que deve
    /// preemptar a corrente: mediana, média e máximo. Zero se não se mediu.
    pub wake_task_median_ns: u64,
    pub wake_task_mean_ns: u64,
    pub wake_task_max_ns: u64,
    /// Custo de uma chamada à política de escalonamento, com 8 tarefas prontas.
    pub pick_next_ns: u64,
    pub tick_ns: u64,
}

/// Exceção que o comando `fault` provoca.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaultTest {
    Breakpoint,
    PageFault,
    /// Escrever em `.text`, que a proteção do kernel deve recusar.
    WriteText,
    /// Pânico numa tarefa de outro núcleo: o sistema todo para e o pânico aparece uma vez.
    CpuPanic,
    /// Page fault com a serial e a tela presas por esta CPU: o relatório não pode ficar mudo.
    ConsoleLock,
    /// Pânico dentro da mensagem do pânico.
    NestedPanic,
    /// Pânico em todas as CPUs ao mesmo tempo.
    Simultaneous,
    /// Ceder a CPU com um lock preso: o escalonador tem de recusar com um pânico.
    HeldLock,
}

/// Retrato do sistema no momento da consulta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemInfo<'a> {
    /// Nome do sistema operacional.
    pub os_name: &'a str,
    /// Versão do sistema operacional.
    pub os_version: &'a str,
    /// ISA em que o kernel roda.
    pub arch: &'a str,
    /// Bootloader que entregou o controle, se ele se identificou.
    pub bootloader: Option<BootloaderId<'a>>,
    /// Marcos do boot, se o bootloader os mediu.
    pub boot_timeline: Option<BootTimeline>,
    /// Nome do processador, se o kernel o descobriu.
    pub cpu: Option<&'a str>,
    /// Memória física.
    pub memory: MemoryInfo,
    /// Tela gráfica, se houver.
    pub display: Option<DisplayInfo>,
}

/// Contabilidade da memória física, em bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryInfo {
    /// RAM que o alocador de frames administra.
    pub usable_bytes: u64,
    /// Parcela de `usable_bytes` ainda livre.
    pub free_bytes: u64,
    /// RAM que o bootloader ou o firmware reservaram.
    pub reserved_bytes: u64,
    /// Metadado do alocador de frames (bitmaps do buddy).
    pub frame_metadata_bytes: u64,
    /// Páginas que o heap do kernel segura do alocador de frames.
    pub heap_pages: u64,
    /// Bytes que o kernel pediu ao heap e ainda não devolveu.
    pub heap_used_bytes: u64,
    /// Tamanho da imagem do kernel.
    pub kernel_bytes: u64,
}

/// Tela gráfica.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayInfo {
    /// Largura em pixels.
    pub width: u32,
    /// Altura em pixels.
    pub height: u32,
    /// Colunas de texto.
    pub cols: u16,
    /// Linhas de texto.
    pub rows: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_name_is_kept() {
        assert_eq!(TaskName::new("count").as_str(), "count");
        assert!(TaskName::default().is_empty() && TaskName::EMPTY.as_str().is_empty());
    }

    #[test]
    fn a_long_name_is_cut_at_the_field_size() {
        let name = TaskName::new("a-program-with-a-very-long-name");
        assert_eq!(name.as_str(), "a-program-with-a");
        assert_eq!(name.as_str().len(), TASK_NAME_BYTES);
    }

    #[test]
    fn the_cut_never_splits_a_character() {
        let name = TaskName::new("123456789012345é");
        assert_eq!(name.as_str(), "123456789012345");
        assert_eq!(TaskName::new("12345678901234é").as_str(), "12345678901234é");
    }
}
