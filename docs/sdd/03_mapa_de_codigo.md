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
│       ├── main.rs             # kmain: boot → memória → console → relatório → boot ok
│       ├── memory.rs           # liga core/mm à memória real pelo mapa direto; janela de dispositivos
│       ├── console.rs          # console global e kprintln!
│       ├── panic.rs            # pânico: escreve o que puder e para a CPU
│       └── platform/           # um módulo por ISA: ARCH, PlatformConsole, cpu(), console()
├── core/                       # o centro do hexágono: no_std, forbid(unsafe_code), testado no host
│   ├── hal/                    # rustos-hal: as portas (traits) e os tipos de fronteira
│   ├── mm/                     # rustos-mm: BitmapFrameAllocator, summarize/find_free_block, AddressWindow
│   └── sched/                  # rustos-sched: TaskId, RunQueue, RoundRobin (ainda não ligado ao kernel)
├── lib/
│   └── sync/                   # rustos-sync: SpinLock (test-and-test-and-set)
├── arch/                       # adaptadores de ISA: o único lugar com asm!
│   ├── x86_64/                 # X86Cpu, PortBus (in/out)
│   └── aarch64/                # Aarch64Cpu, MmioBus, descriptor + walk (puros), Aarch64PageMapper (TTBR1)
├── boot/
│   └── limine/                 # rustos-boot-limine: protocolo Limine rev. 6 escrito aqui → hal::BootInfo
├── drivers/                    # adaptadores de dispositivo: genéricos sobre hal::RegisterBus, sem unsafe
│   ├── uart16550/              # COM1 do PC (porta) ou 16550 em MMIO
│   └── pl011/                  # serial da QEMU virt e de placas ARM
├── xtask/                      # ferramenta de host (só std): build, image, run, smoke, size, check-*
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
| `rustos-sync` | nada |
| `rustos-arch-x86_64` | `rustos-hal` (só no alvo bare-metal) |
| `rustos-arch-aarch64` | `rustos-hal` |
| `rustos-boot-limine` | `rustos-hal` |
| `rustos-driver-uart16550`, `rustos-driver-pl011` | `rustos-hal` |
| `rustos-kernel` | `hal`, `mm`, `sync`, `boot-limine`; por ISA, `arch-*` e o driver da serial |
| `xtask` | nada (só `std`) |

O `rustos-sched` existe com fila e política testadas, mas o kernel ainda não o usa:
ele entra com a troca de contexto, na fase 2 do [roadmap](08_roadmap.md). Por isso
também não está em `[workspace.dependencies]` (o cargo avisa dependência declarada
e não usada).

## Onde ponho isto?

| Código novo | Lugar |
| --- | --- |
| Algoritmo ou política (alocador, escalonador, cache, fila) | crate de `core/` do assunto |
| Contrato que o núcleo precisa do hardware | trait em `core/hal` |
| Instrução privilegiada, registrador de sistema, formato de tabela de página | `arch/<isa>/`; a parte pura (codificação, caminhada) em módulo sem `asm!`, testável no host |
| Protocolo de bootloader | `boot/<protocolo>/` |
| Driver de dispositivo | `drivers/<dispositivo>/`, genérico sobre uma porta do `hal` |
| Primitiva sem domínio que precisa de `unsafe` | `lib/` |
| Qual adaptador atende qual porta numa plataforma | `kernel/src/platform/<isa>.rs` |
| Ferramenta de desenvolvimento | `xtask/` |
