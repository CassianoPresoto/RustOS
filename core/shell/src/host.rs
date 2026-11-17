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

/// Uma linha do `ps`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskInfo<'a> {
    pub id: u32,
    pub name: &'a str,
    pub state: TaskState,
    /// Tempo de CPU acumulado, em nanossegundos.
    pub cpu_ns: u64,
    /// Quantas vezes o escalonador a escolheu para rodar.
    pub switches: u64,
    /// Marcada como usuária de FPU/SIMD: paga o salvamento do estado na troca.
    pub simd: bool,
}

/// Resultado do `bench`: tudo em nanossegundos.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BenchReport {
    /// Custo de uma troca de contexto entre duas tarefas, sem estado de SIMD.
    pub switch_ns: u64,
    /// O mesmo, com as duas tarefas salvando e restaurando FPU/SIMD.
    pub switch_simd_ns: u64,
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
