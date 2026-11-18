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

/// Uma linha do `ps`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskInfo<'a> {
    pub id: u32,
    pub name: &'a str,
    pub state: TaskState,
    /// A CPU dona da tarefa (a fila em que ela está).
    pub cpu: u8,
    /// Tempo de CPU acumulado, em nanossegundos.
    pub cpu_ns: u64,
    /// Quantas vezes o escalonador a escolheu para rodar.
    pub switches: u64,
    /// Marcada como usuária de FPU/SIMD: paga o salvamento do estado na troca.
    pub simd: bool,
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
