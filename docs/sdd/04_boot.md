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
| Executable cmdline | A linha de comando da entrada do `limine.conf`, lida em todo build: `rustos.selftest=1` liga o [modo de teste](#modos-de-boot); com a feature `panic-tests`, `rustos.panic_early=N` (SDD 06). `BootParams::parse` ignora chave e valor que não entende; uma linha que não é ASCII/UTF-8 válido vira `None` e descarta **todos** os parâmetros (o kernel sobe no modo normal; teste em `boot/limine`) | sem linha, o kernel sobe no modo normal |
| MP | Os núcleos: identificador de hardware (LAPIC ID ou `MPIDR`) e o `goto_address` de cada um. No x86_64 o pedido liga o bit de x2APIC. A presença do pedido faz o Limine acordar os núcleos secundários e estacioná-los | sem resposta o kernel roda numa CPU |

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
| FP/SIMD | o alvo `x86_64-unknown-none` não os usa; `task::init` liga x87, SSE e XSAVE | `CPACR_EL1 = 0`; o alvo `-softfloat` não os usa; `task::init` põe `FPEN` em `0b11` ([SDD 15](15_tarefas_e_preempcao.md)) |
| Pilha de execução | `rsp` na pilha do Limine, trocado pelo `call_on_stack` | EL1t com `SP_EL0`; `call_on_stack` passa para EL1h (`SP_EL1`) |

Sem IDT/VBAR, qualquer exceção reinicia (x86) ou trava (aarch64) a máquina. Por isso
a primeira linha do `kmain` instala vetores mínimos ([abaixo](#pânico-e-exceção-antes-do-console));
os definitivos entram em `interrupts::init` ([SDD 10](10_interrupcoes.md)).

## Sequência do `kmain`

0. `early::init()`: vetores de exceção mínimos e a serial sem lock ([abaixo](#pânico-e-exceção-antes-do-console)).
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
   `platform::active_space` lê `TTBR1_EL1` (já o das tabelas próprias), grava
   Device-nGnRnE no índice 7 do `MAIR_EL1`, aloca as tabelas que faltam do alocador de
   frames e mapeia a página na janela de dispositivos.
5. Se há framebuffer, `Screen::new` aloca a grade do terminal em frames contíguos
   (8 bytes por célula), pinta o fundo e a barra do topo, e passa a receber tudo o
   que vai para a serial.
6. `interrupts::init` liga exceções, controlador de interrupção, timer e a IRQ da
   serial ([SDD 10](10_interrupcoes.md)) e destrava as interrupções da CPU no fim. Em seguida
   `smp::start` sobe os núcleos secundários ([abaixo](#núcleos-secundários)).
   Com ela a GDT do Limine sai de uso no x86_64. O timer fica armado para o primeiro
   tique, de um tiro; o tique periódico acaba quando `task::init` liga o escalonador
   ([SDD 15](15_tarefas_e_preempcao.md)).
7. `Memory::reclaim_bootloader` devolve ao buddy as regiões `BootloaderReclaimable`.
8. `random::init` semeia o gerador ([SDD 09](09_seguranca.md)).
9. O que vem depois depende do [modo de boot](#modos-de-boot). **Os dois** imprimem `RustOS`, `boot timeline:`,
   `kaslr:`, `reclaimed:`, `protections:` e `cpus:`, semeiam o gerador (`random::init`, [SDD 09](09_seguranca.md)), rodam o autoteste de um frame (escreve e relê pelo mapa direto), o `w^x:`
   (percorre as tabelas) e o autoteste de exceção, ligam as tarefas (`task::init` adota a execução atual como a tarefa `init`,
   cria a ociosa e liga FPU/SIMD; `smp::release` solta os núcleos), imprimem `boot time:` e `[rustos] boot ok`. **O modo de teste** acrescenta `bootloader:`, `memory:`, `frames:`, `screen:`, `vm:`, `random:`,
   `fpu:`, `sched:`, o autoteste de resíduo de chave (`key residue:`), o do heap, o do timer (`timer:`, cinco tiques
   contra o relógio), `random bench:`, `zero bench:`, a janela `idle:`, o autoteste de preempção e de estado de SIMD
   (`task self-test:`), o de migração (`smp self-test:`) e as medidas curtas (`switch:`, `scale:`, `alloc storm:`).
10. `system::run_shell`: a tarefa `init` vira a tarefa `shell`. Ela lê a fila que a IRQ
   da serial (e, no x86_64, a do teclado) enche e bloqueia (`park`) quando ela está
   vazia; a ociosa dorme em `hlt`/`wfi` quando ninguém está pronto.

Pânico e exceção, em qualquer passo, saem na serial.

## Modos de boot

O kernel tem dois modos, escolhidos pela linha de comando do Limine (`BootParams::parse` em `core/hal/src/cmdline.rs`):

| | Normal (sem linha de comando) | Teste (`rustos.selftest=1`) |
| --- | --- | --- |
| Quem sobe | `make run`, `make screenshot`, o usuário, `make smoke-fast`, `make smoke-panic` | `make smoke` (e o `a64fx` do `make pr`) |
| Autotestes | frame, `w^x:` e exceção: os três juntos custam 3 ms no x86_64 em TCG (com as linhas) e 1 ms no aarch64 com HVF, e nada além deles exercita o que provam (o mapa direto, as tabelas ativas, a IDT/`VBAR` e o retorno da exceção) | os três, mais heap, resíduo de chave, timer, preempção com estado de SIMD e migração entre núcleos |
| Medidas | nenhuma | `zero bench:`, `random bench:`, `idle:`, `switch:`, `scale:`, `alloc storm:` |
| Linhas na serial | as que um gate confere (`kaslr:`, `reclaimed:`, `protections:`, `cpus:`, `w^x:`) ou que avisam (`random: jitter (weak)`) | todas |
| Tempo no kernel (`release`) | 25 a 31 ms (x86_64, TCG) · 2 ms (aarch64, HVF) | 1,0 a 3,8 s conforme o binário (x86_64 em TCG, [DEBT-0068](../tech-debt/0068-boot-de-teste-do-x86-em-tcg-bimodal-por-layout.md)) · 0,45 s (aarch64) |

**Por que dois.** O que mais pesava no boot era espera de relógio e medida, não código de boot: `timer:` (50 ms), `idle:` (50 ms), `task self-test:` (80 ms), `smp self-test:` (140 ms), `switch:`/`scale:`/`alloc storm:` (110 ms no aarch64, 550 ms no x86_64) e, no x86_64 em TCG, o `heap self-test:` (12 ms), o `random bench:` (13 ms) e o `zero bench:` (50 ms). Prova que depende de janela de tempo, ou que mede, não vale a cada boot de quem só quer o shell; vale no `make smoke` e no CI. O que roda sempre é o que custa mil vezes menos e que nada mais exercita (a regra que fixa isso é proposta ao guidelines 05 em Draft separado, à espera do dono).

**Cada linha é tempo.** Na serial da QEMU, uma linha custa de 0,3 ms (x86_64) a 0,4 ms (aarch64) e, com a tela, mais 0,7 ms no x86_64 em TCG (o redesenho da linha). Numa UART física de 115 200 baud seria 7 ms por linha de 80 bytes. O modo normal imprime 11 linhas (12 com o aviso de entropia fraca no aarch64, ou o de calibração do TSC acima de 0,5%), o de teste 31.

**O `bench` do shell** é o mesmo nos dois modos e mede o que o boot de teste mede, com o esforço completo: troca de contexto, IPI, escala, tempestade de alocação, jitter de acordar e, desde esta mudança, o custo de zerar memória e o do gerador aleatório (`Host::micro_bench`).

**O relógio do boot** (`kernel/src/boot_time.rs`) soma, por fase, os ciclos do contador (TSC, `CNTVCT_EL0`) e só converte para tempo no fim, quando o x86_64 já mediu a frequência do TSC contra o PIT. Fases: memória (da entrada do `kmain` até a pilha própria, mais o `reclaim`), console (serial e tela), interrupções (exceções, controlador, calibração do timer), SMP, o resto (`setup`: semente do gerador, linhas de estado, `task::init`), autotestes e medidas. A linha `[rustos] boot time: kernel X ms to prompt (memory A, smp B, self-tests C, benches D)` sai antes de `boot ok`; X é do `kmain` até o `boot ok`, sem o firmware e o Limine (a linha `boot timeline:` os dá). O modo de teste imprime antes dela `boot phases:` com o que a linha não mostra (`console`, `interrupts`, `reclaim`, `setup`, em µs, e no x86_64 o limite do erro da calibração do TSC, `tsc error at most N ppm`).

## Pânico e exceção antes do console

Até o passo 4 não há console, e no aarch64 a PL011 só se alcança depois das tabelas próprias.
O pânico e o relatório de exceção usam, até lá (e quando o lock do console está preso), `early::print`:
escreve direto na serial, sem lock, sem memória e sem tela, e passa pelo mesmo filtro de endereço do
`kprintln!` ([SDD 09](09_seguranca.md#endereços-do-kernel-no-log)).

| | x86_64 | aarch64 |
| --- | --- | --- |
| Vetores (primeira linha do `kmain`) | `install_early`: a IDT estática, com os mesmos stubs de `install`, sobre o seletor de `CS` do Limine; sem IST nem TSS | `install_early`: o `VBAR_EL1` da tabela de sempre, e a pilha que a exceção usa (`SP_EL1` em EL1t, `SP_EL0` em EL1h) 8 KiB abaixo da viva, para o quadro caber; ninguém volta dali |
| Serial | COM1 por `in`/`out`, que funciona desde a entrada; sem programar a UART | PL011 em `TTBR0_EL1` (abaixo) até `Vm::activate`, depois na janela de dispositivos |
| Troca para a definitiva | `install_traps` refaz a IDT sobre a GDT própria | `early_serial_map` mapeia a PL011 no espaço novo **antes** de ativá-lo; `early_serial_switch` aponta a serial precoce para lá e devolve o `TTBR0_EL1` |

**Por que o `TTBR0_EL1` no aarch64.** O HHDM não alcança a PL011: do PROTOCOL.md, para a revisão base 3 em
diante, só `Usable`, `Bootloader reclaimable`, `Executable and modules` e `Framebuffer` entram no HHDM, e a
PL011 (`0x0900_0000`) não está no mapa de memória. Medido na QEMU `virt`: uma escrita em
`HHDM + 0x0900_0000` na entrada do `kmain` trava sem saída.
O mesmo PROTOCOL.md deixa o `TTBR0_EL1` livre para o kernel (revisão 1 em diante), então
`arch/aarch64/src/early.rs` monta nele (níveis 0, 1 e 2) um bloco de 2 MiB de dispositivo, com endereço virtual igual
ao físico, em `0x0900_0000` (múltiplo de 2 MiB); o endereço 0 volta a dar page fault. As três páginas (12 KiB) são um local do `kmain`, na pilha do Limine, que vive até o `early_serial_switch`; o endereço físico delas vem do `AT S1E1R`, e a escrita é volátil porque a MMU as lê fora do compilador e confere que o `TCR_EL1` é
o de 4 níveis do Limine (`T0SZ` 16, `EPD0` 0); sem isso, a serial precoce fica desligada. O `Vm::activate`
liga `EPD0` e o `TTBR0_EL1` volta ao valor do Limine. Alternativas descartadas: semihosting (só a QEMU
com `-semihosting`, e a saída não vai para a serial que o `smoke` lê) e o framebuffer (exige o renderizador
de texto antes de haver memória).

**Pânico dentro do pânico.** `panic::begin` chamado de novo pela CPU dona do pânico toma de volta os
locks do console, e `print_panic` cai para `early::print` quando a serial não está instalada ou está presa.
Só a dona conta (`DEPTH`, depois de `begin`): do terceiro relatório dela em diante, pânico ou falta
(`on_fault` também passa por `begin`), só sai uma linha fixa em `early::print_str` e a CPU para. As outras CPUs
param em silêncio e contam em `STOPPED`, como sempre. O relatório precoce de falta (`early::on_fault`) tem o
mesmo limite: a segunda falta escreve uma linha fixa e a terceira só para.

**Margem de pilha no aarch64** (medida em `dev`, pintando a pilha do Limine de `0xAA` e varrendo: 6 KiB abaixo do quadro do
`kmain` até antes de `Vm::activate`, que já inclui `Memory::init` e `Vm::build`; o quadro tem as 12 KiB das tabelas e
até 4 KiB de alinhamento): cerca de 24 KiB de 64 KiB em uso, margem de uns 40 KiB. No relatório precoce de
falta, 0,5 KiB até o handler mais 1,6 KiB da impressão, de 8 KiB: margem de 6 KiB.

**O que ficou de fora.** Núcleo secundário, até `secondary_init`, ainda usa os vetores e as tabelas do
Limine ([DEBT-0063](../tech-debt/0063-nucleo-secundario-sem-vetores-nem-serial-precoce.md)); a serial precoce
não programa a UART e, no aarch64, é a da QEMU `virt` ([DEBT-0064](../tech-debt/0064-serial-precoce-so-confia-no-firmware.md)).

## Núcleos secundários

O pedido MP faz o Limine acordar cada núcleo e estacioná-lo. O kernel o solta depois de `interrupts::init` e **antes** de `reclaim_bootloader`:
o núcleo ainda anda na pilha e nas tabelas do Limine até trocar as dele.

1. **Antes de soltar** (`smp::start`, na CPU de boot): para cada núcleo que não é o de boot, até `MAX_CPUS` (8; o resto fica parado, com aviso), mapeia a pilha de 32 KiB
   (`Memory::map_stack`) e o que a ISA pede (`platform::prepare_secondary`: 3 pilhas IST e a GDT/TSS no x86_64; a pilha de falhas no aarch64), grava o índice e o `hw_id`, e
   escreve `extra_argument` (o índice) e depois `goto_address`, com release (`boot_limine::start_cpu`).
2. **A entrada** (`ap_entry`, `RDI`/`X0` = `MpInfo`), ainda na pilha do Limine: instala o `PerCpu` (`GS` ou `TPIDR_EL1`), liga as proteções de hardware e **só então** troca para as tabelas do kernel
   (`platform::secondary_early`; no aarch64, o atributo de dispositivo do `MAIR` também). A pilha do Limine segue mapeada: o mapa direto do kernel tem o mesmo deslocamento do HHDM.
3. **Na pilha própria** (`ap_main`): vetores e controlador de interrupção, timer e FPU/SIMD (`platform::secondary_init`, [SDD 10](10_interrupcoes.md)); marca-se online e espera `smp::release`.
4. **O BSP espera** todos ficarem online (3 s no máximo, ou 4 bilhões de voltas se `now_ns` não responder; o que não respondeu entra em `failed` e em `silent`). **Com um núcleo `silent`, `reclaim_bootloader` não roda** (`reclaim: skipped`): ele ainda pode estar na pilha ou nas tabelas do Limine, e a memória não volta ao alocador. O núcleo que falhou no próprio init, já na pilha dele, não entra em `silent` e imprime `[rustos] cpus: N online`. O `make smoke` exige essa linha com o número de núcleos que pediu.
5. **O escalonador** (`task::init`, na CPU de boot) termina, `smp::release` solta o portão e cada núcleo entra em `task::run_secondary`: a execução dele vira a ociosa, e ele passa a receber tarefas ([SDD 15](15_tarefas_e_preempcao.md)).

Falha de um núcleo (sem `MpInfo`, init que erra) deixa `cpus: FAILED mask 0x…` e o boot falha. Sem resposta de MP, o kernel roda numa CPU.

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

QEMU 11.1.3 no macOS (Apple Silicon), 256 MiB, `-smp 4`, perfil `release`, x86_64 em TCG (a base e o mapa direto mudam a cada boot e não saem no log, [SDD 09](09_seguranca.md#endereços-do-kernel-no-log)). Modo normal (`make smoke-fast`):

```text
[rustos] RustOS 0.1.0 (x86_64)
[rustos] boot timeline: firmware 1633044 us, bootloader 265141 us
[rustos] kaslr: on, image slid by up to 18 bits (4 KiB steps in 2 GiB)
[rustos] reclaimed: 43804 KiB from the bootloader
[rustos] protections: +wp +nx +smep +smap +umip
[rustos] cpus: 4 online (found 4, limit 8)
[rustos] frame self-test: ok
[rustos] w^x: ok (65004 pages)
[rustos] exception self-test: ok
[rustos] boot time: kernel 31 ms to prompt (memory 5, smp 1, self-tests 3, benches 0)
[rustos] boot ok
```

Modo de teste (`make smoke`). **Esta saída é do modo lento do x86_64 em TCG** (`switch: 25962 ns`, `kernel 2774 ms`): o tempo das medidas de tarefa nessa plataforma varia até 10x com o tamanho e o alinhamento do código, e o modo rápido leva cerca de 1 s no total ([DEBT-0068](../tech-debt/0068-boot-de-teste-do-x86-em-tcg-bimodal-por-layout.md)). O `make smoke` do x86_64 foi de ~2,7 para ~4,5 s de parede por isso:

```text
[rustos] RustOS 0.1.0 (x86_64)
[rustos] bootloader: Limine 12.10.0
[rustos] boot timeline: firmware 1402005 us, bootloader 258559 us
[rustos] kaslr: on, image slid by up to 18 bits (4 KiB steps in 2 GiB)
[rustos] memory: 255336 KiB usable, 12855480 KiB other, 36 regions
[rustos] reclaimed: 43808 KiB from the bootloader
[rustos] frames: 63608 free of 65244 tracked, metadata 16728 bytes
[rustos] screen: 1280x800, grid 158x47, alt on, history 256 lines
[rustos] protections: +wp +nx +smep +smap +umip
[rustos] vm: direct map in 122 2M and 2451 4K pages, 60 KiB of tables, stack 64 KiB
[rustos] cpus: 4 online (found 4, limit 8)
[rustos] random: rdseed
[rustos] frame self-test: ok
[rustos] w^x: ok (65004 pages)
[rustos] exception self-test: ok
[rustos] key residue: 0 chacha states, 0 key copies on the stack, probe 4 of 4: ok
[rustos] heap self-test: ok
[rustos] timer: 100 Hz, 5 ticks in 48 ms ok
[rustos] random bench: chacha20 113 MB/s, reseed 263 us, next_u64 481.5 ns
[rustos] zero bench: 1 KiB 308.3 ns, 4 KiB 1162.0 ns, 4 KiB over 1 MiB 1219.0 ns, 4 KiB alloc+free 1538.8 ns
[rustos] fpu: xsaveopt, 832 bytes per task, x87 sse avx
[rustos] sched: fifo+eevdf per cpu, 700 us base slice, 24 tasks max, 4 cpus, timer one-shot
[rustos] idle: 1 timer irqs in 50 ms (tickless)
[rustos] task self-test: ok (simd state intact, 27 switches)
[rustos] smp self-test: ok (541 turns, 100 migrations)
[rustos] switch: 25962 ns (1869 with simd, 4582 cross-core), ipi round trip 26174 ns, pick_next 667 ns, tick 93 ns, wake: mean 1338464 ns, max 2020225 ns, 20 samples
[rustos] scale: 8 tasks, 1 cpu: 451929 us, 2 cpu: 372462 us, 4 cpu: 446807 us
[rustos] alloc storm: 4 cpus x 2000, frames locked 138821 us (31491 contended), cached 51872 us (0), heap 91602 us (9344)
[rustos] boot phases: console 3485 us, interrupts 6301 us, reclaim 313 us, setup 11359 us
[rustos] boot time: kernel 2774 ms to prompt (memory 4, smp 1, self-tests 342, benches 2405)
[rustos] boot ok
```

`other` soma toda região que não é utilizável nem recuperável, inclusive faixas reservadas
grandes que não são RAM; não é memória ocupada.
