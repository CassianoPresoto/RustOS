# SDD 04 — Boot

> **O que este capítulo responde:** o caminho do firmware ao `[rustos] boot ok`, o
> que o kernel pede ao bootloader e como fica a memória virtual.

## Cadeia

```text
firmware UEFI (EDK2) → Limine 12.10.0 (BOOTX64.EFI / BOOTAA64.EFI) → kmain (kernel/src/main.rs)
```

O Limine lê `EFI/BOOT/limine.conf` (ao lado do loader EFI, o primeiro lugar onde
procura), carrega o ELF `boot/rustos` e entra em `kmain` com paginação ligada.
Versão e SHA-256 do pacote estão fixados em `xtask/src/limine.rs` (DEBT-0001).

Fonte do protocolo: `PROTOCOL.md` e `include/limine.h` do repositório
[`Limine-Bootloader/limine-protocol`](https://github.com/Limine-Bootloader/limine-protocol).

## O que o kernel pede

Os pedidos ficam em `boot/limine/src/requests.rs`, nas seções `.requests_start_marker`,
`.requests` e `.requests_end_marker`, que o linker script mantém nessa ordem e com
`KEEP`. Com marcadores, o Limine só procura pedidos entre eles.

| Pedido | Para quê | Se faltar |
| --- | --- | --- |
| Revisão base 6 | Estado de máquina e regras de mapeamento atuais (0–5 são obsoletas) | `BootError::UnsupportedBaseRevision`, pânico |
| Memory map | Regiões físicas, ordenadas e alinhadas a 4 KiB | `BootError::MissingMemoryMap`, pânico |
| HHDM | Deslocamento do mapa direto: `virtual = físico + offset` | `BootError::MissingDirectMap`, pânico |
| Bootloader info | Nome e versão para o relatório | linha omitida |
| Bootloader performance | Marcos de reset, início do Limine e entrega ao kernel, em µs | linha omitida |
| Framebuffer | O primeiro framebuffer linear: endereço no HHDM, tamanho, pitch e máscaras de cor. Só 32 bpp RGB com canais de 8 bits é aceito | sem tela; a serial continua |

O Limine mapeia o framebuffer no HHDM com o cache certo para vídeo:
write-combining (`PAT[5]`) no x86_64 e `MAIR_EL1.Attr1` no aarch64 (PROTOCOL.md,
seção *Caching*). O kernel não remapeia: usa o endereço da resposta, e só mexe no
índice 7 do `MAIR_EL1`.

## Estado da máquina na entrada

Do `PROTOCOL.md`, revisão 6:

| | x86_64 | aarch64 |
| --- | --- | --- |
| Privilégio | ring 0, modo longo | EL1, ou EL2 com VHE |
| Interrupções | `IF` zerado; IDT com limite 0 | DAIF mascarado; `VBAR_EL1` indefinido; EL1t (`SP_EL0` em uso) |
| Paginação | 4 níveis; kernel e HHDM mapeados | 4 níveis, grânulo 4 KiB; tabelas da metade alta em `TTBR1_EL1` |
| Pilha | ≥ 64 KiB em memória `BootloaderReclaimable` | idem |
| FP/SIMD | o alvo `x86_64-unknown-none` não os usa | `CPACR_EL1 = 0`; o alvo `-softfloat` não os usa |

Sem IDT/VBAR, qualquer exceção reinicia (x86) ou trava (aarch64) a máquina. A
fase 1 do [roadmap](08_roadmap.md) instala os vetores; o aarch64 está em
[Exceções e interrupções (aarch64)](#exceções-e-interrupções-aarch64).

## Sequência do `kmain`

1. `LimineBoot::from_bootloader()` confere a revisão e lê memory map e HHDM.
2. `Memory::init` resume o mapa (`core/mm::summarize`), calcula o bitmap (1 bit por
   frame do intervalo utilizável), acha onde ele cabe (`find_free_block`), libera
   as regiões utilizáveis e reserva o próprio bitmap.
3. `platform::console` monta a serial. No aarch64 isso mapeia a PL011: o
   `Aarch64PageMapper` lê `TTBR1_EL1`, grava Device-nGnRnE no índice 7 do
   `MAIR_EL1`, aloca as tabelas que faltam do alocador de frames e mapeia a página
   na janela de dispositivos.
4. Se há framebuffer, `Screen::new` aloca a grade do terminal em frames contíguos
   (8 bytes por célula), pinta o fundo e a barra do topo, e passa a receber tudo o
   que vai para a serial.
5. `interrupts::init` liga exceções, controlador de interrupção, timer e a IRQ da
   serial ([SDD 10](10_interrupcoes.md)) e destrava as interrupções da CPU no fim.
6. Relatório (com `protections:`), autoteste de um frame (escreve e relê pelo mapa
   direto), autoteste de exceção e de timer, e `[rustos] boot ok`.
7. `system::run_shell`: o shell embutido lê a fila que a IRQ da serial (e, no
   x86_64, a do teclado) enche, e dorme em `hlt`/`wfi` quando ela está vazia.

Pânico antes do passo 3 não aparece em lugar nenhum (DEBT-0005).

## Memória virtual

| Faixa | O quê | Quem montou |
| --- | --- | --- |
| `0xffff_ffff_8000_0000` + | Imagem do kernel (`.requests`, `.text`, `.rodata`, `.data`/`.bss`, um segmento cada) | Limine, pelo linker script |
| `0xffff_ff00_0000_0000`, 1 GiB | Janela de dispositivos (entrada 510 do nível mais alto), distribuída por `AddressWindow` | kernel |
| `0xffff_fe80_0000_0000`, 1 GiB | Janela das pilhas do kernel (entrada 509), uma página de guarda sem mapeamento embaixo de cada pilha | kernel (`Memory::map_stack`) |
| `HHDM offset` + | Mapa direto de Usable, BootloaderReclaimable, Executable/Modules, Framebuffer e, desde a rev. 4, ACPI e Reserved-mapped | Limine |

O mapa direto do Limine não cobre MMIO nem garante atributo de dispositivo: no
aarch64 ele usa Normal WB (`MAIR` índice 0). Por isso registrador de dispositivo
sempre passa pela janela de dispositivos, nunca pelo HHDM.

## Exceções e interrupções (aarch64)

Fonte dos números: ARM ARM via `esr.h` do Linux (classes e `FSC`), `entry.S` do
Linux (ordem dos vetores), `arm-gic-v3.h` do Linux e `hw/intc/arm_gicv3_*.c` da
QEMU (registradores), `hw/arm/virt.c` (INTIDs e endereços). Cada decisão abaixo
foi conferida nelas.

| Peça | Como é |
| --- | --- |
| Vetores | `VBAR_EL1` aponta para `rustos_vectors` (`arch/aarch64/src/exception.rs`): 2 KiB, 16 entradas de 0x80. O Limine entrega em EL1t, então o kernel é sempre interrompido no grupo "EL1 com SP0"; o grupo SPx usa o mesmo corpo |
| Pilha | `SP_EL1` aponta para uma pilha própria de 4 páginas com guarda (`map_stack`); o kernel segue em `SP_EL0`. Do EL1 só se escreve `SP_EL1` com `SPSel` em 1, então `install` troca `SPSel`, grava o `sp` e volta |
| Quadro | x0–x30, `ELR_EL1`, `SPSR_EL1`: 272 bytes. `brk` volta para a instrução seguinte (o `ELR` aponta para o próprio `brk`: soma 4); falha não recuperável para a CPU em `wfe` |
| `ESR_EL1` | `esr.rs` (puro): classe → `hal::FaultKind`; em aborto, `DFSC`/`IFSC` (tradução por nível, acesso, permissão, alinhamento) e `WnR`. `Fault.code` é o `ESR` cru |
| GICv3 | `drivers/gicv3` (MMIO) mais `gic_cpu.rs` (`ICC_*_EL1`). Distribuidor em `0x0800_0000`, redistribuidor da CPU 0 em `0x080A_0000`, ambos pela janela de dispositivos (DEBT-0002). Sequência: distribuidor desligado → `ARE` + grupo 1 → acorda o redistribuidor (`GICR_WAKER`) → `ICC_SRE_EL1`, `PMR`, `IGRPEN1` |
| Fontes | PPI 27 (`CNTV`, nível) no redistribuidor e SPI 33 (UART0, nível) no distribuidor, grupo 1, roteada para a afinidade do `MPIDR_EL1` desta CPU |
| EOI | `EOImode` 0: um `ICC_EOIR1_EL1` baixa a prioridade e desativa. INTID ≥ 1020 (espúrio) não leva EOI |
| Timer | `CNTV` (o Linux usa o virtual em EL1 sem EL2 e o HVF só entrega o virtual). `CNTFRQ_EL0` lido em tempo de execução. O disparo usa `CNTV_CVAL_EL0` absoluto (`TVAL` acumulava ~2 ms de atraso por tique sob QEMU: 50 tiques em 600 ms) |
| Relógio | `now_ns` = `(CNTVCT − início) × 10⁹ / CNTFRQ` em `u128`; zero em `init_interrupts` |
| PL011 | `IMSC.RXIM \| RTIM`; `ICR` limpo no começo da IRQ, antes de drenar o FIFO: o byte que chega depois rearma a IRQ (limpar depois perdia o acordar no modelo da QEMU) |
| PAN | ligado se `ID_AA64MMFR1_EL1.PAN` ≠ 0, com `SCTLR_EL1.SPAN` = 0 (liga na entrada da exceção) e `PSTATE.PAN` = 1 agora; `protections:` mostra o `PSTATE.PAN` lido de volta |

A mesma configuração roda no HVF (vGIC da Apple, `virt-11.1`+, macOS 15+) e no TCG:
a `virt` fixa o GICv3 por `gic-version=3` e as duas aceleram os mesmos registradores.

## Saída de referência

QEMU 11.1.3 no macOS (Apple Silicon), 256 MiB, perfil `dev`:

```text
[rustos] RustOS 0.1.0 (x86_64)
[rustos] bootloader: Limine 12.10.0
[rustos] boot timeline: firmware 2634227 us, bootloader 350321 us
[rustos] memory: 209232 KiB usable, 12901584 KiB other, 35 regions
[rustos] frames: 52306 free of 65094 tracked, bitmap 8144 bytes
[rustos] frame self-test: ok
[rustos] boot ok
```

`other` soma toda região não utilizável do mapa, inclusive faixas reservadas
grandes que não são RAM; não é memória ocupada.
