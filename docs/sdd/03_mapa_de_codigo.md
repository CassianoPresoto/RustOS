# SDD 03 — Mapa de código

> **O que este capítulo responde:** onde cada tipo de código mora, e quem pode
> depender de quem.

## Árvore

```text
.
├── kernel/                     # rustos-kernel: raiz de composição, o binário `rustos`
│   ├── build.rs                # escolhe o linker script da ISA alvo
│   ├── linker/                 # x86_64.ld, aarch64.ld: metade alta, um segmento por permissão, símbolos de cada seção e PT_DYNAMIC (KASLR)
│   └── src/
│       ├── main.rs             # kmain: boot → memória → serial → tela → relatório → tarefas → boot ok → shell
│       ├── boot.rs             # BootRecord: o que o Limine entregou, copiado para o heap (BootInfo)
│       ├── vm.rs               # Vm: tabelas próprias, W^X por seção, mapa direto, conferência e fault text
│       ├── memory.rs           # Memory::init: buddy e heap; janelas de dispositivos e de pilhas; map_stack; devolução da memória do bootloader
│       ├── frames.rs           # alocador de frames global (IrqLock), o cache de frames por CPU na frente do buddy e o handle Frames
│       ├── heap.rs             # GlobalAlloc sobre KernelHeap, DirectMap (FrameMemory) e o autoteste
│       ├── random.rs           # gerador global: semente de hardware ou jitter, reposição
│       ├── addr.rs             # Addr (`kernel+0x…`) e o filtro de literais `0x…` do log; as regiões que ele nomeia (imagem, mapa direto, janelas); feature `debug-addrs`
│       ├── early.rs            # vetores mínimos e serial sem lock para o pânico e a exceção antes do console (e com o lock preso); `panic-tests`: pontos de falha da linha de comando
│       ├── console.rs          # console global e kprintln!: serial e tela
│       ├── screen.rs           # tela: framebuffer + grade do term + desenho do gfx, tema e barra
│       ├── interrupts.rs       # IRQ do timer (o que o escalonador pede: acordar, fim de fatia, próximo disparo), fila de entrada e o relatório de falha
│       ├── sync.rs             # IrqLock: spinlock que mascara IRQ, conta os presos por CPU e as disputas; `bump` para contador de uma CPU só
│       ├── percpu.rs           # PerCpu: o bloco de cada núcleo (GS/TPIDR_EL1): índice, hw_id, in_irq, locks_held, prazo armado, carga, fila, cache de frames e contadores
│       ├── smp.rs              # núcleos secundários: solta pelo pedido MP, entrada na pilha do Limine, init por CPU, espera do escalonador
│       ├── system.rs           # porta Host do shell (inclui `tasks` e `bench`) e o laço de entrada
│       ├── task.rs             # tarefas do kernel: tabela, uma fila por CPU sobre o `Composite` de `core/sched`, spawn/yield/sleep/park/exit, migração, IPI de reescalonar e a preempção (SDD 15)
│       ├── bench.rs            # autotestes de SIMD e de SMP; troca de contexto (mesma CPU e entre CPUs), IPI de ida e volta, escala, contenção da memória, jitter de acordar e latência de acordar entre tarefas (e o autoteste dela)
│       ├── fmt.rs              # texto formatado em buffer fixo
│       ├── panic.rs            # pânico: o primeiro núcleo para os outros por IPI, toma o console e escreve; o dono refaz o relatório se panicar de novo
│       └── platform/           # um módulo por ISA: ARCH, PlatformConsole, cpu(), console()
├── core/                       # o centro do hexágono: no_std, forbid(unsafe_code), testado no host
│   ├── hal/                    # rustos-hal: as portas (traits), os tipos de fronteira e `redact` (endereço no log), `clock` (`TickScale`, ticks para ns) e `cmdline` (a linha de comando do kernel), puros
│   ├── mm/                     # rustos-mm: BuddyAllocator, FrameCache, KernelHeap (slabs), summarize/find_free_block, AddressWindow, WxChecker, PageTables (a caminhada de 4 níveis das duas ISAs), `runs`/`coalesce` (2 MiB ou 4 KiB no mapa direto); porta FrameMemory e trait FrameAllocator (SDD 12, SDD 13)
│   ├── crypto/                 # rustos-crypto: ChaCha20 (RFC 8439), BLAKE2b (RFC 7693) Rng (fast key erasure) e a semente de reserva por jitter
│   ├── sched/                  # rustos-sched: trait Scheduler, TaskId, RunQueue; RoundRobin, Eevdf (treap de prazos), Fifo e Composite (teto de 95% para o tempo real); sim, property e bench só em teste (SDD 14; o kernel usa o `Composite`)
│   ├── gfx/                    # rustos-gfx: Rgb, PixelFormat, Surface, Style e a fonte própria 8x16
│   ├── term/                   # rustos-term: Terminal (VT/xterm sobre grade de células, tela alternativa, histórico, DEC Special Graphics, tabulação; SDD 16), Cell, Key, Keyboard (layouts)
│   └── shell/                  # rustos-shell: Shell (edição de linha, histórico, comandos) e a porta Host
├── lib/
│   └── sync/                   # rustos-sync: SpinLock (test-and-test-and-set, conta disputas, `force_unlock` para o pânico)
├── arch/                       # adaptadores de ISA: o único lugar com asm!
│   ├── x86_64/                 # X86Cpu, X86CpuLocal (GS), X86Ipi (ICR), PortBus, X86Mmu e DirectMapTables (`invlpg`, `CR3`, memória das tabelas), X86Entropy, GDT/TSS por CPU e IDT única, APIC (periódico → um tiro/TSC-deadline), PIT, troca de pilha, `Context`/`switch`, `fpu`; módulos puros (pte com `X86Format`, segments, fault, lapic, ioapic, pit, clock, xstate) testados no host
│   └── aarch64/                # Aarch64Cpu, Aarch64CpuLocal (TPIDR_EL1), Aarch64Ipi (SGI), MmioBus, Aarch64Mmu e DirectMapTables (TTBR1), `early` (a PL011 por TTBR0 antes das tabelas próprias), Aarch64Entropy; vetores (EL1h), ICC_*, CNTV de um tiro, PAN, `Context`/`switch` e `fpu`;
│                               # puros: descriptor (com `Aarch64Format`), esr, time, intid, sgi, features
├── boot/
│   └── limine/                 # rustos-boot-limine: protocolo Limine rev. 6 (inclusive MP) escrito aqui → hal::BootInfo
├── drivers/                    # adaptadores de dispositivo: genéricos sobre hal::RegisterBus, sem unsafe
│   ├── uart16550/              # COM1 do PC (porta) ou 16550 em MMIO; recepção por consulta ou interrupção
│   ├── pl011/                  # serial da QEMU virt e de placas ARM (com a IRQ de recepção)
│   ├── i8042/                  # teclado PS/2 do PC: controlador e decodificador do conjunto 1
│   └── gicv3/                  # GICv3 da ARM: distribuidor, redistribuidor (achado por afinidade) e SGI, sobre hal::RegisterBus
├── xtask/                      # ferramenta de host (só std): build, image, run, smoke, screenshot, size, font-preview, check-*
├── docs/                       # sdd/, guidelines/, tech-debt/
├── .github/workflows/ci.yml    # gates, testes e smoke nas duas ISAs
├── Makefile                    # atalhos para cargo e xtask
├── rust-toolchain.toml         # Rust estável fixado + alvos bare-metal
└── .cargo/config.toml          # alias `cargo xtask`, relocation-model=pie nos alvos (KASLR)
```

## Quem depende de quem

| Crate | Depende de |
| --- | --- |
| `rustos-hal` | nada |
| `rustos-mm` | `rustos-hal` |
| `rustos-crypto` | nada |
| `rustos-sched` | nada |
| `rustos-gfx` | nada |
| `rustos-term` | `rustos-hal` |
| `rustos-shell` | `rustos-hal` |
| `rustos-sync` | nada |
| `rustos-arch-x86_64` | `rustos-hal` (só no alvo bare-metal) |
| `rustos-arch-aarch64` | `rustos-hal` |
| `rustos-boot-limine` | `rustos-hal` |
| `rustos-driver-uart16550`, `rustos-driver-pl011`, `rustos-driver-i8042`, `rustos-driver-gicv3` | `rustos-hal` |
| `rustos-kernel` | `hal`, `mm`, `crypto`, `sched`, `gfx`, `term`, `shell`, `sync`, `boot-limine`; por ISA, `arch-*` e os drivers (x86_64: `uart16550`, `i8042`; aarch64: `pl011`, `gicv3`) |
| `xtask` | nada (só `std`) |

O `rustos-sched` é usado pelo `kernel/src/task.rs` desde a fase 3: o `Composite` (FIFO e
EEVDF) escolhe a próxima tarefa e o kernel guarda o estado dela ([SDD 14](14_escalonador.md),
[SDD 15](15_tarefas_e_preempcao.md)).

## Onde ponho isto?

| Código novo | Lugar |
| --- | --- |
| Algoritmo ou política (alocador, escalonador, cache, fila) | crate de `core/` do assunto |
| Estado por CPU (fila, carga, contadores) | `kernel/src/percpu.rs`; o registrador que o acha (`GS`, `TPIDR_EL1`) é da porta `hal::CpuLocal`, em `arch/` |
| Subir um núcleo, o IPI | `kernel/src/smp.rs`, `task.rs`; o ICR e a SGI em `arch/` (porta `hal::Ipi`) |
| Estado de uma tarefa, pilha, troca de contexto, preempção, migração | `kernel/src/task.rs`; salvar e restaurar registrador, em `arch/<isa>/src/context.rs` e `fpu.rs` |
| Cifra, hash ou gerador de números aleatórios | `core/crypto/`; a semente do hardware vem por porta do `hal` |
| Desenho de pixel, glifo da fonte | `core/gfx/` |
| Sequência de terminal, codificação de tecla, layout de teclado | `core/term/` |
| Comando do shell embutido | `core/shell/`; o dado do sistema chega pela porta `Host` |
| Tema, barra do topo, como a grade vira pixel | `kernel/src/screen.rs` |
| Contrato que o núcleo precisa do hardware | trait em `core/hal` |
| Instrução privilegiada, registrador de sistema, formato de tabela de página | `arch/<isa>/`; a parte pura (codificação, caminhada) em módulo sem `asm!`, testável no host |
| Protocolo de bootloader | `boot/<protocolo>/` |
| Driver de dispositivo | `drivers/<dispositivo>/`, genérico sobre uma porta do `hal` |
| Primitiva sem domínio que precisa de `unsafe` | `lib/` |
| Qual adaptador atende qual porta numa plataforma | `kernel/src/platform/<isa>.rs` |
| Ferramenta de desenvolvimento | `xtask/` |
