# SDD 02 — Arquitetura

> **O que este capítulo responde:** como o kernel se divide em centro, portas e
> adaptadores, e por onde passa uma chamada do núcleo ao hardware.

## O hexágono

| Anel | Pasta | O que tem | `unsafe` |
| --- | --- | --- | --- |
| **Portas** | `core/hal` | Traits e tipos de fronteira: `BootInfo`, `Console`, `Cpu`, `RegisterBus`, `PageMapper`, `FrameSource`, `PhysAddr`, `VirtAddr`, `MemoryRegion` | proibido |
| **Centro** | `core/mm`, `core/sched` | Política pura: alocador de frames, leitura do mapa de memória, janela de endereços, fila de execução, rodízio | proibido |
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
| `Aarch64PageMapper` | `Aarch64PageMapper::active(offset)` | as tabelas ativas e os frames novos são acessíveis pelo mapa direto |
| `LimineBoot` | `LimineBoot::from_bootloader()` | a memória das respostas do Limine ainda não foi reaproveitada |
| `X86Cpu`, `Aarch64Cpu` | `::new()` | o código roda em ring 0 / EL1+ |

Depois disso, `Uart16550<PortBus>` e `Pl011<MmioBus>` são drivers sem uma linha
de `unsafe`, genéricos sobre `hal::RegisterBus`.

## O caminho de uma chamada

`kprintln!` no `kernel/` → `console::print` → `SpinLock<PlatformConsole>` →
`hal::Console::write_bytes` → `Pl011<MmioBus>::write_byte` (driver, seguro) →
`hal::RegisterBus::write32` → `MmioBus` (adaptador, `write_volatile`) → PL011.

No x86_64 é o mesmo caminho com `Uart16550<PortBus>` e a instrução `out`.

## Portas de entrada que virão

Hoje toda porta é de saída (o núcleo chama o hardware). Na fase 4, a entrada de
syscall é a primeira porta de entrada; na fase 5, a ABI do Linux é um adaptador de
entrada que traduz syscalls Linux para os serviços do núcleo, no modelo do
Asterinas (`kernel/core/src/syscall/`) — ver [SDD 08](08_roadmap.md).
