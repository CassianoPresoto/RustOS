# SDD 03 — Mapa de código

> **O que este capítulo responde:** onde cada tipo de código mora, e quem pode
> depender de quem.

## Árvore

```text
.
├── kernel/                     # rustos-kernel: raiz de composição, o binário `rustos`
│   ├── build.rs                # escolhe o linker script da ISA alvo
│   ├── linker/                 # x86_64.ld, aarch64.ld: metade alta, um segmento por permissão
│   └── src/
│       ├── main.rs             # kmain: boot → memória → serial → tela → relatório → boot ok → shell
│       ├── memory.rs           # liga core/mm à memória real pelo mapa direto; janela de dispositivos
│       ├── console.rs          # console global e kprintln!: serial e tela
│       ├── screen.rs           # tela: framebuffer + grade do term + desenho do gfx, tema e barra
│       ├── system.rs           # porta Host do shell e o laço de entrada
│       ├── fmt.rs              # texto formatado em buffer fixo
│       ├── panic.rs            # pânico: escreve o que puder e para a CPU
│       └── platform/           # um módulo por ISA: ARCH, PlatformConsole, cpu(), console()
├── core/                       # o centro do hexágono: no_std, forbid(unsafe_code), testado no host
│   ├── hal/                    # rustos-hal: as portas (traits) e os tipos de fronteira
│   ├── mm/                     # rustos-mm: BitmapFrameAllocator, summarize/find_free_block, AddressWindow
│   ├── sched/                  # rustos-sched: TaskId, RunQueue, RoundRobin (ainda não ligado ao kernel)
│   ├── gfx/                    # rustos-gfx: Rgb, PixelFormat, Surface, Style e a fonte própria 8x16
│   ├── term/                   # rustos-term: Terminal (VT/xterm sobre grade de células), Cell, Key, Keyboard (layouts)
│   └── shell/                  # rustos-shell: Shell (edição de linha, histórico, comandos) e a porta Host
├── lib/
│   └── sync/                   # rustos-sync: SpinLock (test-and-test-and-set)
├── arch/                       # adaptadores de ISA: o único lugar com asm!
│   ├── x86_64/                 # X86Cpu, PortBus, X86PageMapper, GDT/TSS/IDT, APIC, PIT; módulos puros (pte, walk, segments, fault, lapic, ioapic, pit, clock) testados no host
│   └── aarch64/                # Aarch64Cpu, MmioBus, Aarch64PageMapper (TTBR1); vetores, ICC_*, CNTV e PAN;
│                               # puros: descriptor, walk, esr, time, intid, features
├── boot/
│   └── limine/                 # rustos-boot-limine: protocolo Limine rev. 6 escrito aqui → hal::BootInfo
├── drivers/                    # adaptadores de dispositivo: genéricos sobre hal::RegisterBus, sem unsafe
│   ├── uart16550/              # COM1 do PC (porta) ou 16550 em MMIO; recepção por consulta ou interrupção
│   ├── pl011/                  # serial da QEMU virt e de placas ARM (com a IRQ de recepção)
│   ├── i8042/                  # teclado PS/2 do PC: controlador e decodificador do conjunto 1
│   └── gicv3/                  # GICv3 da ARM: distribuidor e redistribuidor, sobre hal::RegisterBus
├── xtask/                      # ferramenta de host (só std): build, image, run, smoke, screenshot, size, font-preview, check-*
├── docs/                       # sdd/, guidelines/, tech-debt/
├── .github/workflows/ci.yml    # gates, testes e smoke nas duas ISAs
├── Makefile                    # atalhos para cargo e xtask
├── rust-toolchain.toml         # Rust estável fixado + alvos bare-metal
└── .cargo/config.toml          # alias `cargo xtask`, relocation-model=static nos alvos
```

## Quem depende de quem

| Crate | Depende de |
| --- | --- |
| `rustos-hal` | nada |
| `rustos-mm` | `rustos-hal` |
| `rustos-sched` | nada |
| `rustos-gfx` | nada |
| `rustos-term` | `rustos-hal` |
| `rustos-shell` | `rustos-hal` |
| `rustos-sync` | nada |
| `rustos-arch-x86_64` | `rustos-hal` (só no alvo bare-metal) |
| `rustos-arch-aarch64` | `rustos-hal` |
| `rustos-boot-limine` | `rustos-hal` |
| `rustos-driver-uart16550`, `rustos-driver-pl011`, `rustos-driver-i8042`, `rustos-driver-gicv3` | `rustos-hal` |
| `rustos-kernel` | `hal`, `mm`, `gfx`, `term`, `shell`, `sync`, `boot-limine`; por ISA, `arch-*` e os drivers (x86_64: `uart16550`, `i8042`; aarch64: `pl011`, `gicv3`) |
| `xtask` | nada (só `std`) |

O `rustos-sched` existe com fila e política testadas, mas o kernel ainda não o usa:
ele entra com a troca de contexto, na fase 3 do [roadmap](08_roadmap.md). Por isso
também não está em `[workspace.dependencies]` (o cargo avisa dependência declarada
e não usada).

## Onde ponho isto?

| Código novo | Lugar |
| --- | --- |
| Algoritmo ou política (alocador, escalonador, cache, fila) | crate de `core/` do assunto |
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
