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
│       ├── main.rs             # kmain: boot → memória → serial → tela → relatório → tarefas → boot ok → shell; modo normal ou de teste (`rustos.selftest=1`, só na imagem com a feature `selftest`); `boot_time.rs`: o relógio por fase do boot
│       ├── boot.rs             # BootRecord: o que o Limine entregou (inclusive os módulos), copiado para o heap (BootInfo)
│       ├── vm.rs               # Vm: tabelas próprias, W^X por seção, mapa direto, conferência e fault text
│       ├── memory.rs           # Memory::init: buddy e heap; janelas de dispositivos e de pilhas; map_stack; devolução da memória do bootloader
│       ├── frames.rs           # alocador de frames global (IrqLock), o cache de frames por CPU na frente do buddy e o handle Frames
│       ├── heap.rs             # GlobalAlloc sobre KernelHeap e DirectMap (FrameMemory); `heap/selftest.rs`: o autoteste e a medida de zerar (feature `selftest`)
│       ├── random.rs           # gerador global: semente de hardware ou jitter, reposição; `random/selftest.rs`: a medida e o resíduo de chave (feature `selftest`)
│       ├── addr.rs             # Addr (`kernel+0x…`) e o filtro de literais `0x…` do log; as regiões que ele nomeia (imagem, mapa direto, janelas); feature `debug-addrs`
│       ├── early.rs            # vetores mínimos e serial sem lock para o pânico e a exceção antes do console (e com o lock preso); `panic-tests`: pontos de falha da linha de comando
│       ├── console.rs          # console global e kprintln!: serial e tela
│       ├── screen.rs           # tela: framebuffer + grade do term + desenho do gfx, tema e barra
│       ├── interrupts.rs       # IRQ do timer (o que o escalonador pede: acordar, fim de fatia, próximo disparo), fila de entrada e o relatório de falha
│       ├── stack.rs            # marca d'água das pilhas: pinta com um padrão e lê o pico varrendo do fundo (SDD 15)
│       ├── sync.rs             # IrqLock: spinlock que mascara IRQ, conta os presos por CPU e as disputas; `bump` para contador de uma CPU só
│       ├── percpu.rs           # PerCpu: o bloco de cada núcleo (GS/TPIDR_EL1): índice, hw_id, in_irq, locks_held, prazo armado, carga, fila, cache de frames e contadores
│       ├── smp.rs              # núcleos secundários: solta pelo pedido MP, entrada na pilha do Limine, init por CPU, espera do escalonador
│       ├── system.rs           # porta Host do shell (inclui `tasks` e `bench`) e o laço de entrada
│       ├── task.rs             # tarefas do kernel: tabela, uma fila por CPU sobre o `Composite` de `core/sched`, spawn/yield/sleep/park/exit, migração, IPI de reescalonar e a preempção (SDD 15)
│       ├── selftest.rs         # o modo de teste do boot: as linhas extras, os autotestes e as medidas (feature `selftest`); sem a feature, `selftest_off.rs` recusa o `rustos.selftest=1` e não faz mais nada
│       ├── bench.rs            # (feature `selftest`) autotestes de SIMD e de SMP; troca de contexto (mesma CPU e entre CPUs), IPI de ida e volta, escala, contenção da memória, jitter de acordar e latência de acordar entre tarefas (e o autoteste dela)
│       ├── fmt.rs              # texto formatado em buffer fixo
│       ├── panic.rs            # pânico: o primeiro núcleo para os outros por IPI, toma o console e escreve; o dono refaz o relatório se panicar de novo; `fault cpu` e, com `panic-tests`, os outros `fault`
│       └── platform/           # um módulo por ISA: ARCH, PlatformConsole, cpu(), console()
├── core/                       # o centro do hexágono: no_std, forbid(unsafe_code), testado no host
│   ├── hal/                    # rustos-hal: as portas (traits), os tipos de fronteira e `redact` (endereço no log), `clock` (`TickScale`, ticks para ns) e `cmdline` (a linha de comando do kernel), puros
│   ├── mm/                     # rustos-mm: BuddyAllocator, FrameCache, KernelHeap (slabs), summarize/find_free_block, AddressWindow, WxChecker, PageTables (a caminhada de 4 níveis das duas ISAs), first_unpainted (a marca d'água da pilha), `runs`/`coalesce` (2 MiB ou 4 KiB no mapa direto), `vmar` (árvore de regiões do espaço de usuário: reserva, alocação com ASLR, `unmap`, a decisão da falta de página; SDD 17) e `asid` (gerador de PCID/ASID com geração entre CPUs; SDD 17); porta FrameMemory e trait FrameAllocator (SDD 12, SDD 13)
│   ├── crypto/                 # rustos-crypto: ChaCha20 (RFC 8439), BLAKE2b (RFC 7693) Rng (fast key erasure) e a semente de reserva por jitter
│   ├── sched/                  # rustos-sched: trait Scheduler, TaskId, RunQueue; RoundRobin, Eevdf (treap de prazos), Fifo e Composite (teto de 95% para o tempo real); sim, property e bench só em teste (SDD 14; o kernel usa o `Composite`)
│   ├── gfx/                    # rustos-gfx: Rgb, PixelFormat, Surface, Style e a fonte própria 8x16
│   ├── term/                   # rustos-term: Terminal (VT/xterm sobre grade de células, tela alternativa, histórico, DEC Special Graphics, tabulação; SDD 16), Cell, Key, Keyboard (layouts)
│   ├── cpio/                   # rustos-cpio: leitor de cpio `newc` sem alocação (iterador sobre `&[u8]`), para o initramfs; só arquivo regular sai
│   ├── elf/                    # rustos-elf: validação de ELF64 estático (`ET_EXEC`) para uma `Image` sem alocação; o chamador passa os `Limits` (ISA e faixa do usuário)
│   └── shell/                  # rustos-shell: Shell (edição de linha, histórico, comandos) e a porta Host; o comando `bench` (`bench.rs`) é da feature `bench`, que o `selftest` do kernel liga
├── lib/
│   └── sync/                   # rustos-sync: SpinLock (test-and-test-and-set, conta disputas, `force_unlock` para o pânico)
├── arch/                       # adaptadores de ISA: o único lugar com asm!
│   ├── x86_64/                 # X86Cpu, X86CpuLocal (GS), X86Ipi (ICR), PortBus, X86Mmu e DirectMapTables (`invlpg`, `CR3`, memória das tabelas), X86Entropy, GDT/TSS por CPU e IDT única, APIC (periódico → um tiro/TSC-deadline), PIT, troca de pilha, `Context`/`switch`, `fpu`; módulos puros (pte com `X86Format`, segments, fault, lapic, ioapic, pit, clock, xstate) testados no host
│   └── aarch64/                # Aarch64Cpu, Aarch64CpuLocal (TPIDR_EL1), Aarch64Ipi (SGI), MmioBus, Aarch64Mmu e DirectMapTables (TTBR1), `early` (a PL011 por TTBR0 antes das tabelas próprias), Aarch64Entropy; vetores (EL1h), ICC_*, CNTV de um tiro, PAN, `Context`/`switch` e `fpu`;
│                               # puros: descriptor (com `Aarch64Format`), esr, time, intid, sgi, features
├── boot/
│   └── limine/                 # rustos-boot-limine: protocolo Limine rev. 6 (inclusive MP e módulos) escrito aqui → hal::BootInfo
├── drivers/                    # adaptadores de dispositivo: genéricos sobre hal::RegisterBus, sem unsafe
│   ├── uart16550/              # COM1 do PC (porta) ou 16550 em MMIO; recepção por consulta ou interrupção
│   ├── pl011/                  # serial da QEMU virt e de placas ARM (com a IRQ de recepção)
│   ├── i8042/                  # teclado PS/2 do PC: controlador e decodificador do conjunto 1
│   └── gicv3/                  # GICv3 da ARM: distribuidor, redistribuidor (achado por afinidade) e SGI, sobre hal::RegisterBus
├── xtask/                      # ferramenta de host (só std): build, image, run, smoke, screenshot, size, font-preview, initramfs, check-*
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
| `rustos-cpio` | nada |
| `rustos-elf` | nada |
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
| Ler o initramfs (cpio `newc`) | `core/cpio/` |
| Validar um executável ELF e dizer o que mapear | `core/elf/`; os limites da ISA e da faixa do usuário vêm do chamador (`elf::Limits`) |
| Comando do shell embutido | `core/shell/`; o dado do sistema chega pela porta `Host` |
| Tema, barra do topo, como a grade vira pixel | `kernel/src/screen.rs` |
| Contrato que o núcleo precisa do hardware | trait em `core/hal` |
| Instrução privilegiada, registrador de sistema, formato de tabela de página | `arch/<isa>/`; a parte pura (codificação, caminhada) em módulo sem `asm!`, testável no host |
| Protocolo de bootloader | `boot/<protocolo>/` |
| Driver de dispositivo | `drivers/<dispositivo>/`, genérico sobre uma porta do `hal` |
| Primitiva sem domínio que precisa de `unsafe` | `lib/` |
| Qual adaptador atende qual porta numa plataforma | `kernel/src/platform/<isa>.rs` |
| Ferramenta de desenvolvimento | `xtask/` |

## Formatos de entrada: `core/cpio` e `core/elf`

Os dois leem bytes que vêm de fora do kernel (o initramfs do bootloader e o executável
dentro dele) e por isso são `forbid(unsafe_code)`, sem alocação e sem dependência: a
entrada hostil vira `Error` (enum, uma variante por regra), nunca pânico. Todo cálculo
de tamanho e de offset é `checked_*`. Os testes de host ([SDD 06](06_testes.md)) têm um
caso por regra de recusa, as truncações em todos os comprimentos e os campos nos extremos.

Nenhum dos dois está em `[workspace.dependencies]` ainda: o primeiro consumidor (o carregador da 4b, no `kernel/`) acrescenta a entrada junto com a dependência.

**`core/cpio`** (`Reader`, `Entry`, `find`). Regras do `newc` como no
[early-userspace buffer format](https://docs.kernel.org/driver-api/early-userspace/buffer-format.html):
magic `070701`, 13 campos de 8 dígitos hexadecimais, cabeçalho de 110 bytes, registro e
dados alinhados a 4, avanço por `c_namesize`, `TRAILER!!!` **exigido**. Decisões do agente:

| Caso | Decisão |
| --- | --- |
| Diretório sem dado | **Pulado**: o `cpio -o` alimentado por `find` gera um por pasta, e recusar tornaria o initramfs comum inválido |
| Diretório com dado, link simbólico, dispositivo, FIFO e socket | Recusados (`DirectoryWithData`, `Symlink`, `SpecialFile`) |
| Arquivo regular com `c_nlink` diferente de 1 | Recusado (`LinkCount`): hard link e contagem zero |
| `070702` (com soma) | Recusado (`ChecksumFormat`); o `070701` não tem soma e o RustOS não confere a dos outros |
| Hexadecimal | Maiúscula e minúscula aceitas; qualquer outro caractere é `BadHex` |
| `c_namesize` | De 1 a 4096 (`PATH_MAX`); o nome termina no primeiro NUL dentro dele; nome vazio é `BadName` |
| Bytes depois do `TRAILER!!!` | Ignorados (o preenchimento até 512 do `cpio` está aí) |
| `find(name)` | Compara byte a byte (sem tirar `./`: o escritor do xtask grava `init`), percorre o arquivo **inteiro** (o `TRAILER!!!` vale mesmo com o achado no começo) e, com nome repetido, fica com a **última** ocorrência (proposta; a extração em ordem do Linux sobrescreve, **não verificado no código** nesta fatia) |

O teste de ouro é um `newc` real gerado no desenvolvimento por
`printf 'init\nbin\nbin/tool\n' | cpio -o -H newc` (bsdcpio 3.5.3 do macOS) e colado como constante;
o binário `cpio` não entra no build.

**`core/elf`** (`parse(&[u8], &Limits) -> Result<Image, Error>`). `Image` tem a entrada, até
16 `PT_LOAD` (`Segment`: `vaddr`, `memsz`, `offset`, `filesz`, `flags`), o endereço da
tabela de program headers na memória (`phdr`, só se ela cabe no conteúdo de arquivo de um
`PT_LOAD`) e `phnum`. **Só `ET_EXEC`**; `ET_DYN` volta `BadType` até a decisão 5 do
capítulo 18 mudar. As regras são a tabela "Validação, regra por regra" do capítulo 17.
**Mudança de dependência:** o crate **não** depende do `hal`. O chamador passa
`Limits { machine, user_min, user_top, page_size, max_loads_bytes }`, e o `kernel/` os tira de
`hal::abi`. Decisões do agente onde a tabela deixa margem:

| Caso | Decisão |
| --- | --- |
| `p_align` | 0 e 1 (sem alinhamento, gABI) e potência de dois passam; o resto é `BadAlign`. A congruência é módulo a **página**, não módulo `p_align` |
| `e_entry` | Dentro do **conteúdo de arquivo** (`p_filesz`) de um `PT_LOAD` executável; a cauda zerada (`.bss`) não vale |
| `e_version` | Além de `EI_VERSION`, `e_version` também tem de ser 1 |
| Ordem dos `PT_LOAD` | `p_vaddr` menor que o do anterior é `Unsorted`; mesma página ou invasão, `Overlap` |
| Teto de memória | Soma dos `p_memsz` (sem arredondar) até `max_loads_bytes` |
| `PT_LOAD` sem R | Recusado também quando só W, só X ou nenhum |
| Tipos ignorados | `PT_NULL`, `PT_NOTE`, `PT_PHDR`, `PT_GNU_*` e os desconhecidos não são validados |
| Sem nenhum `PT_LOAD` | `EntryNotExecutable` |
| `Segment` | `flags` guarda só `PF_R`, `PF_W` e `PF_X` (os bits de SO e de processador de `p_flags` saem). `page_range(page)` dá o intervalo virtual arredondado à página e `page_offset(page)` o deslocamento de `vaddr` na página. `file_bytes` começa em `vaddr`, não no início da página: o chamador zera o começo da primeira página e a cauda até `memsz` |

`EI_OSABI` tem de ser `ELFOSABI_NONE` (0). **Achado nos testes:** o `rust-lld` grava
`ELFOSABI_GNU` (3) quando o programa tem um `#[used]` (seção com `SHF_GNU_RETAIN`); o mesmo
programa sem ele sai com 0. O `user/rt` e os programas não podem usar `#[used]`, ou a regra
muda (contrato de formato: caso 6 do [`AGENTS.md`](../../AGENTS.md); [DEBT-0074](../tech-debt/0074-elf-recusa-osabi-gnu-do-used.md)).

O ELF real dos testes é um `#![no_std] #![no_main]` mínimo ligado com
`rustc 1.100.0 --target {x86_64-unknown-none,aarch64-unknown-none-softfloat} -C opt-level=s
-C panic=abort [-Ccode-model=small no x86_64] -C relocation-model=static -C link-arg=-Tuser.ld
-C link-arg=-zmax-page-size=0x1000 -C link-arg=--no-rosegment`, com `.text` em `0x40_0000` e
uma página por permissão. As constantes são só o cabeçalho e os três program headers; o resto do arquivo
o teste preenche com zeros. O programa não entra no build.
