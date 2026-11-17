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
| Executable address | Base física e virtual da imagem (com o sorteio do KASLR), para o kernel mapear cada seção | `BootError::MissingKernelAddress`, pânico |
| Framebuffer | O primeiro framebuffer linear: endereço no HHDM, tamanho, pitch e máscaras de cor. Só 32 bpp RGB com canais de 8 bits é aceito | sem tela; a serial continua |

O Limine mapeia o framebuffer no HHDM com o cache certo para vídeo:
write-combining (`PAT[5]`) no x86_64 e `MAIR_EL1.Attr1` no aarch64 (PROTOCOL.md,
seção *Caching*). O kernel usa o endereço da resposta e, nas tabelas próprias, o
mapeia com o mesmo cache (`PageFlags::FRAMEBUFFER`, [SDD 13](13_memoria_virtual.md)); só
escreve o índice 7 do `MAIR_EL1`. Responde-se uma vez: depois do `BootRecord` nada lê
a resposta do Limine.

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

1. `LimineBoot::from_bootloader()` confere a revisão e lê memory map, HHDM e endereço
   da imagem.
2. `Memory::init` resume o mapa (`core/mm::summarize`), calcula o metadado do buddy
   (`BuddyAllocator::words_needed`, 2 bits por frame do intervalo utilizável ou
   recuperável), acha onde ele cabe (`find_free_block`), libera as regiões
   utilizáveis, reserva o próprio metadado e instala o alocador de frames e o heap
   globais ([SDD 12](12_memoria_fisica_e_heap.md)).
3. `BootRecord::capture` copia o que o Limine entregou para o heap. `platform::harden`
   liga as proteções da CPU. `Vm::build` monta as tabelas próprias e `Vm::activate`
   troca `CR3` / `TTBR1_EL1`. `Memory::map_stack` cria a pilha de 64 KiB e
   `call_on_stack` passa a rodar nela ([SDD 13](13_memoria_virtual.md)).
4. `platform::console` monta a serial. No aarch64 isso mapeia a PL011: o
   `Aarch64PageMapper` lê `TTBR1_EL1` (já o das tabelas próprias), grava
   Device-nGnRnE no índice 7 do `MAIR_EL1`, aloca as tabelas que faltam do alocador de
   frames e mapeia a página na janela de dispositivos.
5. Se há framebuffer, `Screen::new` aloca a grade do terminal em frames contíguos
   (8 bytes por célula), pinta o fundo e a barra do topo, e passa a receber tudo o
   que vai para a serial.
6. `interrupts::init` liga exceções, controlador de interrupção, timer e a IRQ da
   serial ([SDD 10](10_interrupcoes.md)) e destrava as interrupções da CPU no fim.
   Com ela a GDT do Limine sai de uso no x86_64.
7. `Memory::reclaim_bootloader` devolve ao buddy as regiões `BootloaderReclaimable`.
8. `random::init` semeia o gerador ([SDD 09](09_seguranca.md)).
9. Relatório (com `kernel base:`, `reclaimed:`, `vm:`, `protections:`, `random:` e
   `random bench:`), autoteste de um frame (escreve e relê pelo mapa direto), autoteste
   do heap, `w^x:` (percorre as tabelas), autoteste de exceção e de timer, e
   `[rustos] boot ok`.
10. `system::run_shell`: o shell embutido lê a fila que a IRQ da serial (e, no
   x86_64, a do teclado) enche, e dorme em `hlt`/`wfi` quando ela está vazia.

Pânico antes do passo 4 não aparece em lugar nenhum (DEBT-0005).

## Memória virtual

Tabelas do kernel desde o passo 3; o detalhe, as permissões e as medidas estão no
[SDD 13](13_memoria_virtual.md).

| Faixa | O quê | Quem montou |
| --- | --- | --- |
| `0xffffffff80000000`..: base sorteada (KASLR) | Imagem do kernel, uma permissão por seção | Limine carrega e relocaliza; o kernel mapeia de novo |
| `HHDM offset` + (sorteado) | Mapa direto de `Usable` e `BootloaderReclaimable`, leitura e escrita sem execução; framebuffer com write-combining | kernel (`Vm::build`); o Limine só diz o deslocamento |
| 1 entrada do nível mais alto, escolhida no boot | Janela de dispositivos (1 GiB) e janela das pilhas do kernel (1 GiB), uma página de guarda sem mapeamento embaixo de cada pilha | kernel (`Memory::place_windows`, `map_stack`) |
| Metade baixa | Vazia | kernel |

O mapa direto do kernel não cobre MMIO nem a imagem do kernel. Por isso registrador de
dispositivo sempre passa pela janela de dispositivos, nunca pelo HHDM.

## Exceções e interrupções

O que cada ISA liga em `platform::init_interrupts`, com vetores, controlador,
timer e fontes, está no [SDD 10](10_interrupcoes.md).

## Saída de referência

QEMU 11.1.3 no macOS (Apple Silicon), 256 MiB, perfil `dev` (a base e o mapa direto mudam a cada boot):

```text
[rustos] RustOS 0.1.0 (x86_64)
[rustos] bootloader: Limine 12.10.0
[rustos] boot timeline: firmware 1295365 us, bootloader 242971 us
[rustos] kernel base: 0xffffffff9b7a5000 (physical 0xe092000)
[rustos] memory: 255532 KiB usable, 12855284 KiB other, 35 regions
[rustos] reclaimed: 46432 KiB from the bootloader
[rustos] frames: 63811 free of 65268 tracked, metadata 16728 bytes
[rustos] screen: 1280x800, grid 158x47
[rustos] protections: +wp +nx +smep +smap +umip
[rustos] vm: direct map at 0xffff846840000000 in 122 2M and 2475 4K pages, 64 KiB of tables, stack 64 KiB
[rustos] random: rdseed
[rustos] random bench: chacha20 39 MB/s, reseed 288 us
[rustos] frame self-test: ok
[rustos] heap self-test: ok
[rustos] w^x: ok (64968 pages)
[rustos] exception self-test: ok
[rustos] timer: 100 Hz, 5 ticks in 37 ms ok
[rustos] boot ok
```

`other` soma toda região que não é utilizável nem recuperável, inclusive faixas reservadas
grandes que não são RAM; não é memória ocupada.
