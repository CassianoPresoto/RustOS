# SDD 02 — Arquitetura

> **O que este capítulo responde:** como o kernel se divide em centro, portas e
> adaptadores, e por onde passa uma chamada do núcleo ao hardware.

## O hexágono

| Anel | Pasta | O que tem | `unsafe` |
| --- | --- | --- | --- |
| **Portas** | `core/hal` | Traits e tipos de fronteira: `BootInfo` (com `cpus` e `modules`), `Console`, `ByteSource`, `Cpu`, `CpuLocal`, `Ipi`, `RegisterBus`, `PageMapper`, `FrameSource`, `PageFormat`, `TableMemory`, `MmuControl`, `HardwareEntropy`, `FramebufferInfo`, `PhysAddr`, `VirtAddr`, `MemoryRegion` | proibido |
| **Centro** | `core/mm`, `core/crypto`, `core/sched`, `core/gfx`, `core/term`, `core/shell`, `core/cpio`, `core/elf` | Política pura: alocadores de frames (buddy, cache por CPU) e do heap, leitura do mapa de memória, janela de endereços, fila de execução, rodízio (o estado das tarefas e a troca moram no `kernel/` e no `arch/`); ChaCha20, BLAKE2b e o gerador de números aleatórios; desenho de pixels e a fonte própria; emulador de terminal VT/xterm; shell embutido; leitor de cpio `newc` e validador de ELF64 ([SDD 03](03_mapa_de_codigo.md#formatos-de-entrada-corecpio-e-coreelf)) | proibido |
| **Suporte** | `lib/sync` | Primitivas sem domínio (`SpinLock`) que precisam de `unsafe` para existir | permitido |
| **Adaptadores** | `arch/*`, `boot/*`, `drivers/*` | Implementam as portas para uma ISA, um bootloader ou um dispositivo | permitido em `arch/` e `boot/`; proibido em `drivers/` |
| **Raiz de composição** | `kernel/` | Entrada, pânico, console global, e a escolha de qual adaptador atende cada porta | permitido |
| **Ferramenta** | `xtask/` | Host: build, imagem, QEMU, medição e os gates | — |

Dependência só aponta para dentro: o centro conhece as portas; o adaptador conhece
as portas e o suporte; só o `kernel/` conhece adaptadores. A regra inteira, e o
que o `make layers-check` confere, está no
[guidelines 03](../guidelines/03_camadas_e_dependencias.md).

## Por que assim

- **Trocar hardware não toca política.** Uma ISA nova é um crate em `arch/` e um
  módulo em `kernel/src/platform/`. Um bootloader novo é um crate em `boot/` que
  devolve `hal::BootInfo`. `core/` não muda.
- **Política testada no host, sem QEMU.** Tudo em `core/` e os drivers rodam
  `cargo test` no Mac ou no Linux, com o barramento de registradores ou a memória
  de tabelas falsos. Bug de alocador ou de caminhada de tabela aparece em
  milissegundos, não num boot que trava.
- **`unsafe` encurralado.** É a divisão que o Asterinas chama de *framekernel*:
  um núcleo pequeno com `unsafe`, o resto em Rust seguro
  ([ATC '25](https://www.usenix.org/conference/atc25/presentation/peng-yuke)). O
  Tock separa do mesmo jeito: traits de HIL no kernel, implementações em `chips/`,
  composição em `boards/` ([TRD 2](https://github.com/tock/tock/blob/master/doc/reference/trd2-hil-design.md)).

## Como o `unsafe` vira API segura

A unsafety mora no **construtor** do adaptador, onde o invariante é estabelecido;
os métodos da porta são seguros. Exemplos:

| Adaptador | Construtor `unsafe` | O que quem chama garante |
| --- | --- | --- |
| `PortBus` (x86_64) | `PortBus::new(base)` | as portas a partir de `base` são do dispositivo e ninguém mais as usa |
| `MmioBus` (aarch64) | `MmioBus::new(virt)` | `virt` está mapeado como dispositivo e cobre o bloco de registradores |
| `DirectMapTables` (as duas ISAs) | `DirectMapTables::new(offset)` | toda tabela e todo frame novo é acessível pelo mapa direto, e ninguém mais mexe nas tabelas |
| `platform::active_space`, `platform::new_space` | `active_space(offset)`, `new_space(frames, offset)` | as tabelas ativas e os frames novos são acessíveis pelo mapa direto; só se ativa o espaço novo com código, pilha e mapa direto nele |
| `LimineBoot` | `LimineBoot::from_bootloader()` | a memória das respostas do Limine ainda não foi reaproveitada |
| `X86Cpu`, `Aarch64Cpu`, `X86CpuLocal`, `Aarch64CpuLocal` | `::new()` | o código roda em ring 0 / EL1+ |
| `X86Ipi`, `Aarch64Ipi` | `::new()` | o Local APIC publicado (x86_64) ou a interface do GIC desta CPU ligada (aarch64) |
| `Context::new` (as duas ISAs) | `Context::new(top, entry, arg, on_return)` | `top` é o fim, alinhado a 16, de uma pilha mapeada só da thread; `switch` só roda com IRQ mascarada e nenhum lock preso |
| `DirectMap` (`kernel/src/heap.rs`) | `DirectMap::new(offset)` | todo endereço físico que a porta `FrameMemory` recebe é de frame do kernel e está no mapa direto, em `físico + offset` |

Depois disso, `Uart16550<PortBus>` e `Pl011<MmioBus>` são drivers sem uma linha
de `unsafe`, genéricos sobre `hal::RegisterBus`.

## O caminho de uma chamada

`kprintln!` no `kernel/` → `console::print` → `SpinLock<PlatformConsole>` →
`hal::Console::write_bytes` → `Pl011<MmioBus>::write_byte` (driver, seguro) →
`hal::RegisterBus::write32` → `MmioBus` (adaptador, `write_volatile`) → PL011.

No x86_64 é o mesmo caminho com `Uart16550<PortBus>` e a instrução `out`.

## O caminho da tela e do shell

A tela não tem biblioteca de terceiros em nenhuma camada: a fonte, o emulador de
terminal, o desenho e o shell são deste repositório.

| Passo | Quem | O que faz |
| --- | --- | --- |
| 1 | `kernel/src/console.rs` (`Sink`) | Manda cada texto para a serial e para a tela, cada uma com o próprio lock |
| 2 | `kernel/src/screen.rs` (`Screen::write`) | Traduz `\n` em `\r\n` (o `onlcr` de um tty) e entrega os bytes ao terminal |
| 3 | `core/term` (`Terminal::write`) | Interpreta UTF-8 e as sequências VT/xterm sobre a grade de células, e marca as linhas sujas |
| 4 | `kernel/src/screen.rs` (`render`) | Para cada célula das linhas sujas: cor pelo tema, estilo (`gfx::Style`) e glifo (`gfx::font`); células sem tinta e da mesma cor viram um retângulo só (`fill_rect`). `Screen::new` pinta o fundo e descarta as linhas sujas iniciais: o primeiro `render` não repinta uma grade vazia |
| 5 | `core/gfx` (`Surface::draw_glyph`) | Escreve os pixels em ordem, linha a linha, sem nunca ler o framebuffer |

Com a tela alternativa e o histórico, a grade que `render` lê é a da visão: a
principal, a alternativa ou, rolada para trás, linhas do histórico ([SDD 16](16_terminal.md)).
Shift+PageUp e Shift+PageDown só marcam a visão suja em `Screen::key_event`; o `flush` do laço do shell desenha, fora da IRQ.

A grade do terminal é o buffer-sombra: quem precisa saber o que está na tela lê a
grade, nunca a memória de vídeo, que no x86_64 é write-combining e lenta de ler.

A entrada faz o caminho inverso: `hal::ByteSource` (a UART) → `Shell::feed` →
comando, que consulta o sistema pela porta `rustos_shell::Host`, implementada em
`kernel/src/system.rs`. O shell define a própria porta (`Host`) porque ela só
existe para ele; o `kernel/` é quem a liga ao `mm`, ao boot e à tela.

O shell calcula as quebras de linha pela largura da tela (`Host::terminal_size`)
e manda a mesma saída para a serial. Um terminal de host mais estreito que a grade
desenha mal as linhas longas editadas; a tela é o alvo.

O `Sink` só atualiza a grade; `console::flush` desenha uma vez por mensagem do
`kprintln!` e uma vez por tecla do shell, porque cada desenho repinta linhas
inteiras.

O `crypto` também não conhece o hardware: a semente (RDSEED, RNDR) entra pela
porta `hal::HardwareEntropy`, que `arch/x86_64` e `arch/aarch64` implementam, e o
estado do gerador mora no `kernel/`, atrás de um lock (`kernel/src/random.rs`,
[SDD 09](09_seguranca.md)).

`gfx`, `term` e `shell` não se conhecem (guidelines 03, regra 2): `term` não sabe
de pixels, `gfx` não sabe de terminal, e o `shell` só escreve texto em
`core::fmt::Write`.

## O caminho de um `Vec` no kernel

`Vec::push` → `GlobalAlloc::alloc` (`kernel/src/heap.rs`) → lock `HEAP` →
`KernelHeap::allocate` (`core/mm`, política pura) → `FrameAllocator::allocate_contiguous`
da porta, implementada por `Frames` → lock `FRAMES` → `BuddyAllocator` (`core/mm`). O
`PhysAddr` volta pelo mesmo caminho, e o `alloc` soma o deslocamento do mapa direto.
Quem toca a memória é só o `DirectMap`, a implementação de `FrameMemory`. Ordem de
lock e detalhes no [SDD 12](12_memoria_fisica_e_heap.md).

## O caminho de uma interrupção (aarch64)

`ICC_IAR1_EL1` (`arch/aarch64/src/gic_cpu.rs`) dentro da entrada de vetor
(`exception.rs`) → `hal::Handlers::irq(Irq(intid))` → `interrupts::on_irq` →
`platform::irq_source` traduz o INTID em `IrqSource` → o tratador da fonte (o
timer, ou drenar a UART para a fila de entrada) → `ICC_EOIR1_EL1` →
`hal::Handlers::preempt`. A ociosa dorme em `wfi` quando ninguém está pronto.
Detalhes em [SDD 04](04_boot.md).

## O caminho de uma troca de contexto

Timer dispara → `interrupts::on_timer` (`task::timer_irq`: acordar quem dorme, `tick`
da política, rearmar o disparo que ela pede) → o `arch/` dá o EOI e chama
`Handlers::preempt` (`task::preempt`) na pilha da tarefa interrompida → se o `Composite`
de `core/sched` pede troca (`needs_resched`), `schedule()` pede a ele a próxima tarefa
(`pick_next`) → `platform::fpu_save`/`fpu_restore` (só tarefa marcada SIMD) →
`arch/<isa>::switch` troca os registradores callee-saved e o `sp`. O mesmo `switch_from`
atende `yield_now`, `sleep_until`, `park` e `exit`, e `unpark` e `spawn` o chamam ao fim quando
a acordada deve preemptar a corrente. Desenho completo no [SDD 15](15_tarefas_e_preempcao.md).

## Portas de entrada que virão

Hoje toda porta é de saída (o núcleo chama o hardware). Na fase 4, a entrada de
syscall é a primeira porta de entrada; na fase 5, a ABI do Linux é um adaptador de
entrada que traduz syscalls Linux para os serviços do núcleo, no modelo do
Asterinas (`kernel/core/src/syscall/`) — ver [SDD 08](08_roadmap.md).
