# SDD 10 — Exceções, interrupções e tempo no x86_64

> **O que este capítulo responde:** como o kernel recebe exceção e IRQ no x86_64,
> de onde vem o tique e o relógio, e que proteções de hardware ligam no boot.

## Sequência de `platform::init_interrupts`

| Passo | O quê | Onde |
| --- | --- | --- |
| 1 | WP (`CR0`), SMEP, SMAP, UMIP (`CR4`) e NXE (`EFER`), só o que o CPUID anuncia; o estado lido de volta vira `protections()` | `arch/x86_64/src/protect.rs` |
| 2 | `X86PageMapper::active`: lê o `CR3` e caminha as tabelas pelo mapa direto | `mmu.rs`, `walk.rs`, `pte.rs` |
| 3 | Três pilhas de 4 páginas com página de guarda (`Memory::map_stack`), para #DF, NMI e #MC | `kernel/src/memory.rs` |
| 4 | GDT, TSS e IDT próprias; `lgdt`, CS por retorno distante, `ltr`, `lidt` | `traps.rs`, `segments.rs` |
| 5 | 8259 mascarado (0xFF em 0x21 e 0xA1) | `pic.rs` |
| 6 | Local APIC: x2APIC por MSR se `CPUID.1:ECX[21]`, senão xAPIC por MMIO na janela de dispositivos | `apic.rs`, `lapic.rs` |
| 7 | I/O APIC em `0xFEC0_0000` (DEBT-0009): pino 4 (COM1) → vetor `0x24`; pino 1 (teclado) → `0x21`, mascarado | `ioapic.rs` |
| 8 | Calibração: um intervalo de 20 ms do PIT (canal 2) mede o TSC e o contador do timer do APIC; timer periódico em `TIMER_HZ` | `pit.rs`, `clock.rs` |
| 9 | `Uart16550::enable_receive_interrupt` (`IER` bit 0 e `OUT2` do `MCR`), por último | `drivers/uart16550` |

O chamador (`interrupts::init`) liga o `IF` depois. A parte pura de cada passo
roda no `cargo test` do host, com `RegisterBus` e memória de tabelas falsos.

## Vetores

| Vetor | Quem | Pilha |
| --- | --- | --- |
| 0–31 | Exceções da CPU; tem código de erro 8, 10–14, 17, 21, 29 e 30 (`fault::ERROR_CODE_VECTORS`) | atual |
| 8 (#DF), 2 (NMI), 18 (#MC) | IST1, IST2 e IST3 da TSS (o campo do gate é o índice mais um) | própria |
| `0x20` | Timer do Local APIC | atual |
| `0x21` | Teclado PS/2 (pino 1, mascarado) | atual |
| `0x24` | COM1 (pino 4) | atual |
| `0xFF` | Espúrio: chega ao `Handlers.irq` e não recebe EOI | atual |

Os 256 gates apontam para stubs de 16 bytes (`rustos_isr_stubs + 16 * vetor`) que
empilham um código de erro falso onde a CPU não empilha e o número do vetor. A
entrada comum salva os 15 registradores (ordem de `TrapFrame`), faz `cld` e chama
`trap_dispatch`, que decodifica em `hal::Fault` (page fault pelos bits do código de
erro e pelo `CR2`) ou entrega `hal::Irq` ao núcleo e dá o EOI depois do handler. O
breakpoint volta; qualquer outra exceção pára a CPU.

## Seletores

| Seletor | Descritor |
| --- | --- |
| `0x08` | Código do kernel (64 bits) |
| `0x10` | Dados do kernel |
| `0x18` (RPL 3) | Dados do usuário |
| `0x20` (RPL 3) | Código do usuário (dados antes do código, a ordem do `sysret`) |
| `0x28` | TSS de 16 bytes (ocupa duas entradas) |

## Tempo

O tique é o timer do Local APIC em modo periódico, divisor 16, contagem inicial
`apic_hz / TIMER_HZ`. O relógio (`now_ns`) é o TSC menos a leitura da calibração,
convertido por `ticks * 1e9 / tsc_hz` em `u128`, sem ponto flutuante. Sob a QEMU
`-cpu max` em TCG não há TSC-deadline nem TSC invariante, e o vendor é AMD, o que
inutiliza o CPUID 0x15 do Linux: por isso a referência é o PIT.

| Medida (QEMU 11.1.3, TCG, `make smoke`) | Valor |
| --- | --- |
| TSC | ~1,0000 GHz (o TCG conta nanossegundos) |
| Contador do timer do APIC, divisor 16 | ~62,5 MHz |
| `timer: 100 Hz, 5 ticks in …` | 40 a 45 ms (a fase do primeiro tique é aleatória) |

## Custo

`make size ARCH=x86_64 PROFILE=release`, antes e depois: carregado 99 301 → 118 976
bytes (`text` 80 415 → 94 591, `bss` 6 → 4 504). Os 256 stubs são 4 096 bytes de
`text` e a IDT, 4 096 de `bss`; o resto é o despacho, o APIC e a calibração.

## Memória virtual acrescentada

| Faixa | O quê |
| --- | --- |
| `0xffff_fe80_0000_0000`, 1 GiB | Janela de pilhas (entrada 509), com página de guarda abaixo de cada pilha |
| `0xffff_ff00_0000_0000`, 1 GiB | Janela de dispositivos (entrada 510): Local APIC (xAPIC) e I/O APIC, com `PCD\|PWT` (PAT[3] = UC no layout do Limine) |

## Fontes

| Afirmação | Fonte |
| --- | --- |
| Vetores com código de erro | `exception_has_error_code` em `target/i386/tcg/seg_helper.c` da QEMU (8, 10–14, 17) e `arch/x86/kernel/traps.c`, `idtentry.h` do Linux (também 21, 29); a tabela 6-1 da SDM vol. 3A não foi lida direto |
| Bits do código de erro do #PF | `arch/x86/include/asm/trap_pf.h` do Linux |
| x2APIC: `CPUID.1:ECX[21]`, `IA32_APIC_BASE` EN (bit 11) e EXTD (bit 10) | `CPUID_EXT_X2APIC` e `MSR_IA32_APICBASE_*` em `target/i386/cpu.h` da QEMU; `X2APIC_ENABLE` em `apicdef.h` do Linux |
| Registradores do APIC e MSR x2APIC `0x800 + (deslocamento >> 4)` | `arch/x86/include/asm/apicdef.h` do Linux |
| Calibração pelo PIT (porta 0x61, canal 2, `0xB0`) | `pit_calibrate_tsc` em `arch/x86/kernel/tsc.c` do Linux; porta 0x61 em `hw/audio/pcspk.c` da QEMU (bit 5 é o OUT2) |
| Frequência do PIT 1 193 182 Hz | `PIT_FREQ` em `include/hw/timer/i8254.h` da QEMU |
| I/O APIC em `0xFEC0_0000` | `IO_APIC_DEFAULT_ADDRESS` em `include/hw/intc/ioapic.h` da QEMU |
| PAT do Limine (PAT3 = UC) | `PROTOCOL.md` do Limine, seção *Caching* |
| IST no gate é índice mais um | comentário em `arch/x86/kernel/idt.c` do Linux |
