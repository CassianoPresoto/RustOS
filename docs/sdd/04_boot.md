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
| Interrupções | `IF` zerado; IDT com limite 0 | DAIF mascarado; `VBAR_EL1` indefinido |
| Paginação | 4 níveis; kernel e HHDM mapeados | 4 níveis, grânulo 4 KiB; tabelas da metade alta em `TTBR1_EL1` |
| Pilha | ≥ 64 KiB em memória `BootloaderReclaimable` | idem |
| FP/SIMD | o alvo `x86_64-unknown-none` não os usa | `CPACR_EL1 = 0`; o alvo `-softfloat` não os usa |

Sem IDT/VBAR, qualquer exceção reinicia (x86) ou trava (aarch64) a máquina. A
fase 1 do [roadmap](08_roadmap.md) instala os vetores.

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
5. `interrupts::init` liga exceções, controlador, timer e a IRQ da serial (no
   x86_64, [SDD 10](10_interrupcoes_x86_64.md)) e destrava o `IF`.
6. Relatório (inclui `protections`), autoteste de um frame (escreve e relê pelo
   mapa direto), autoteste de exceção e de timer, e `[rustos] boot ok`.
7. `system::run_shell`: o shell embutido lê a fila que a IRQ da serial enche e dorme
   em `hlt` quando está vazia. Sem interrupções, a leitura é por consulta (DEBT-0007).

Pânico antes do passo 3 não aparece em lugar nenhum (DEBT-0005).

## Memória virtual

| Faixa | O quê | Quem montou |
| --- | --- | --- |
| `0xffff_ffff_8000_0000` + | Imagem do kernel (`.requests`, `.text`, `.rodata`, `.data`/`.bss`, um segmento cada) | Limine, pelo linker script |
| `0xffff_ff00_0000_0000`, 1 GiB | Janela de dispositivos (entrada 510 do nível mais alto), distribuída por `AddressWindow` | kernel |
| `HHDM offset` + | Mapa direto de Usable, BootloaderReclaimable, Executable/Modules, Framebuffer e, desde a rev. 4, ACPI e Reserved-mapped | Limine |

O mapa direto do Limine não cobre MMIO nem garante atributo de dispositivo: no
aarch64 ele usa Normal WB (`MAIR` índice 0). Por isso registrador de dispositivo
sempre passa pela janela de dispositivos, nunca pelo HHDM.

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
