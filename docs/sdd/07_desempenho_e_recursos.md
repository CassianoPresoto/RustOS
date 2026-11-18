# SDD 07 — Desempenho e recursos

> **O que este capítulo responde:** o que já se mede, como se vai comparar com Linux
> e Windows, e quais técnicas a pesquisa aponta para cada subsistema.

## O que já se mede

| Métrica | Como | Valor hoje (QEMU, 256 MiB) |
| --- | --- | --- |
| Kernel carregado | `make size PROFILE=release` / `release-small` | x86_64: 316 048 / 188 004 bytes · aarch64: 264 329 / 169 165 bytes (a otimização da troca e do acordar custou +7 536 / +2 472 e +4 604 / +2 296 sobre os 308 512 / 185 532 e 259 725 / 166 869 seguintes: o ponto de preempção de `unpark`, `TickScale`, a medida e o autoteste de acordar entre tarefas e o aquecimento do `bench`; o `.bss` ficou igual, as amostras vão para a pilha do acordador; o `sort_unstable` da `core` custava 3 KiB e virou uma inserção). Antes dela: x86_64: 308 512 / 185 532 bytes · aarch64: 259 725 / 166 869 bytes (o SMP da fase 3 custou +47 024 / +26 512 e +38 148 / +25 088 sobre os 261 488 / 159 020 e 221 577 / 141 781 que seguem, sem ele: fila por CPU, migração, IPI, pânico, cache de frames e o `bench` de SMP; o `.bss` do SMP é de poucos KiB, porque a política de cada CPU vem do heap). Antes do SMP: x86_64: 261 488 / 159 020 bytes · aarch64: 221 577 / 141 781 bytes (sobre 250 448 / 155 388 e 213 693 / 138 573 da `main` `85ae25e`, mesmo comando: +11 040 / +3 632 e +7 884 / +3 208. É o zerar por `DC ZVA`, o autoteste de faixas e a linha `zero bench:`, mais o resíduo de chave (`Words`, o autoteste `key residue:` com a sonda e o `next_u64` do `random bench:`); boa parte é a formatação das duas linhas de medida). Antes, fase 3 com o EEVDF no kernel: +24 584 / +12 136 (x86_64) e +20 520 / +11 824 (aarch64) sobre o mesmo kernel com o rodízio; `.data` do `SCHED`, 3 968 bytes; o resto é a política, o `i128` do EEVDF e a medida da política no `bench`). Antes, a fase 3 só com o rodízio, tarefas, preempção, estado de SIMD, timer de um tiro e `ps` e `bench`: 211 096 / 135 356 e 182 421 / 120 201, medidos de novo no commit anterior (+26 763 / +13 856 e +29 440 / +16 956 sobre a fase 2, que a linha a seguir descreve). Antes, a fase 2 depois da revisão do PR 5: 184 093 / 121 404 e 153 101 / 103 129 (`.data.rel.ro` só leitura, caminhada única em `core/mm`, bit de livre no slab). Antes da revisão: 183 669 / 120 004 e 155 889 / 102 021. Antes: com buddy, heap e gerador, 166 544 / 106 095 e 136 292 / 88 364; antes do buddy, 133 304 / 88 735 e 105 448 / 71 508; na fase 0.5, 100 541 / 61 725 e 86 500 / 59 952. O PIE do KASLR custa 3 289 / 5 581 (x86_64) e 5 361 / 6 265 (aarch64), de 1,8% a 6,6% ([SDD 13](13_memoria_virtual.md)) |
| Memória livre depois do boot (o "pronto quando" da fase 2) | `frames:` do boot (QEMU 256 MiB, `dev`) e `mem` | x86_64: 63 811 frames (249,3 MiB, 97,4% da RAM), antes 52 321 (204,4 MiB) · aarch64: 63 237 (247,0 MiB, 96,5%), antes 54 991 (214,8 MiB). Devolvido: 46 432 e 33 424 KiB |
| Tabelas de página do kernel | `vm:` do boot | 60 a 64 KiB (x86_64) e 44 a 48 KiB (aarch64) com 2 MiB no mapa direto; 548 e 536 KiB só com 4 KiB. Varia com o deslocamento sorteado do mapa direto |
| Grade do terminal | 8 bytes por célula (`size_of::<Cell>()`), em frames contíguos | x86_64 1280x800: 158x47 células, 59 408 bytes (15 frames) · aarch64 1024x768: 126x45, 45 360 bytes (12 frames) |
| Tela alternativa do terminal | Uma segunda grade, do mesmo tamanho da principal, entregue pelo kernel (`set_alternate`, [SDD 16](16_terminal.md)) | 8 bytes por célula: x86_64 59 408 bytes (15 frames) · aarch64 45 360 bytes (12 frames) |
| Histórico de rolagem do terminal | Anel de linhas inteiras (`set_history`): `cols * 8` bytes por linha, `HISTORY_LINES = 256` em `kernel/src/screen.rs`; gravar uma linha é um `copy_from_slice` de `cols` células. Só com teclado: o aarch64 da `virt` não aloca (`history 0 lines` no boot, [DEBT-0050](../tech-debt/0050-historico-sem-rolagem-no-aarch64.md)) | x86_64 1 264 bytes por linha, 323 584 no total (79 frames) · aarch64 1 008 por linha, 258 048 que ficam sem alocar enquanto não houver teclado |
| CPU do histórico de rolagem | Medido pela revisão do PR: `release`, host, melhor de 7, com 256 linhas de histórico contra nenhum | 20 mil linhas de 158 colunas rolando: +5% a +10%, cerca de 15 ns por linha rolada; nas outras cargas, dentro do ruído |
| Redesenho da tela cheia | `console::flush` com tempo de `now_ns` em volta, `colors` três vezes, `release`, maior valor visto (medida descartável, não está no código) | x86_64 (QEMU TCG): 27,7 ms · aarch64 (QEMU com HVF): 1,5 ms. É o custo de Shift+PageUp e de qualquer linha nova com a tela cheia; roda com IRQ mascarada ([DEBT-0051](../tech-debt/0051-redesenho-da-tela-com-irq-mascarada.md)). Número de TCG só compara o RustOS consigo mesmo |
| Custo da tela alternativa, do histórico e das caixas no boot | `frames:` do boot (QEMU 256 MiB, `dev`) e `make size`, na `main` e com a mudança | Frames livres, x86_64: 63 801 → 63 704 (97 frames, 388 KiB, com o histórico) · aarch64: 63 227 → 63 211 (16 frames, 64 KiB, só a grade alternativa). Kernel `release` / `release-small`: x86_64 236 320 → 251 200 (+14 880) / 147 732 → 155 388 (+7 656) · aarch64 203 729 → 213 693 (+9 964) / 132 297 → 138 569 (+6 272). Inclui `alt`, `lines` e os 9 glifos novos |
| Fonte própria | tabelas em `.rodata` (teste em `core/gfx/src/font`) | 4 046 bytes para 246 glifos 8x16 (3 884 para 237 antes dos 9 glifos do DEC Special Graphics: ◆ ≤ ≥ π ≠ ⎺ ⎻ ⎼ ⎽) |
| Shell | `size_of::<Shell<256, 32>>()`, na pilha do `kmain` | 8 880 bytes |
| Metadado de memória física | `BuddyAllocator::words_needed` (SDD 12): um bitmap por ordem com resumo hierárquico; é o `metadata` da linha `frames:` do boot e o `frame meta` do `mem` | 16 704 bytes para 65 094 frames (x86_64) e 16 504 para 64 396 (aarch64): 2,05 bits por frame, 0,008% da RAM; 2,03 bits por frame em 4 GiB ou mais. O bitmap de antes, que saiu do kernel: 8 144 e 8 056 bytes |
| Heap do kernel | `heap::self_test` e a linha `heap` do `mem` | Metadado de 64 bytes por slab, dentro da página, com o mapa de livres de 1 bit por objeto (SDD 12). Zerar ao liberar ligado: custo abaixo |
| Zerar página | linha `zero bench:` do boot (`heap::zero_benchmark`), `make smoke ARCH=… PROFILE=release` | aarch64 HVF: 4 KiB em 20 a 38 ns (antes 124 a 169), 1 KiB em 13 a 16 (antes 36 a 47); x86_64 TCG: 1,1 µs por 4 KiB, sem mudança (tabela no heap, acima) |
| Aleatório no kernel | linha `random bench:` do boot (`random::benchmark`): 32 blocos de 4 KiB com `Rng::fill_bytes` e uma reposição completa, com o relógio da fase 1 | aarch64 sob HVF, `release`: 636 a 694 MB/s e 36 a 38 us por reposição (tabela abaixo) |
| Boot até o kernel | linha `boot timeline` (pedido *bootloader performance* do Limine) | Limine: 52 ms (aarch64, HVF) / 350 ms (x86_64, TCG) |
| Pilha de falhas (aarch64) | `Memory::map_stack`: 4 páginas mapeadas e uma de guarda, por CPU; é o `SP_EL0` do EL1h (SDD 15) | 16 384 bytes de RAM, mais as tabelas de página que faltarem |
| Custo por CPU | SDD 15, "Custo por tarefa e por CPU": `PerCpu`, a política no heap, a pilha e as pilhas de exceção | ~500 bytes de `PerCpu` (`.bss`, por CPU possível) mais 3 680 de política no heap por CPU que entra no escalonador; núcleo secundário: 32 KiB de pilha mais 48 KiB de IST (x86_64) ou 16 KiB de pilha de falhas (aarch64); cache de frames: 256 bytes |
| Custo por tarefa | SDD 15, "Custo por tarefa e por CPU": `Slot`, `Context`, pilha e área de FPU/SIMD | ~140 bytes de `Slot` e 96 de entidade na política justa; 8 (x86_64) ou 104 (aarch64) de `Context`; 32 768 de pilha (mais as tabelas de página); 832 (x86_64 da QEMU, `xsaveopt` com AVX) ou 528 (aarch64) de FPU/SIMD, só da tarefa que usa |
| Troca de contexto | `bench` e a linha `switch:`: ping-pong de `unpark`/`park` entre duas tarefas, ns por troca, aquecimento e melhor de 3 | aarch64 HVF `release`, 5 boots: **41 a 44 ns** na linha do boot (antes 68 a 75) e **42 ns** no comando (antes 73 a 158); a tabela e a decomposição estão em "Otimização da troca e do acordar", abaixo; x86_64 e aarch64 em TCG só para comparar o RustOS consigo mesmo |
| Jitter de acordar | `bench`: `agora − pedido` depois de `sleep_until`, 200 amostras | aarch64 HVF, com o EEVDF: 0,49 a 0,52 ms (ociosa em `wfi`) e 4,1 µs (CPU ocupada) de média; igual ao do rodízio (tabela abaixo) |
| Latência de acordar entre tarefas | `bench` (linha `task to task`) e a linha `wake self-test:` do boot: `unpark` de uma tarefa de fatia curta por outra, na mesma CPU, até ela rodar  | aarch64 HVF: mediana **42 a 83 ns** (um a dois tiques do contador de 24 MHz), antes 102 µs (o fim da fatia da corrente) |
| Timer (aarch64) | linha `timer:` do boot; `CNTV` com `CVAL` absoluto, 100 Hz só até `task::init`; depois não há tique periódico (linha `idle:`) | 50 tiques em 497 ms (HVF) e 488 ms (TCG) no autoteste; ociosa com o EEVDF: 2 IRQs do timer em 50 ms, contra 5 a 100 Hz (HVF, TCG e x86_64 iguais). No x86_64 com o contador do APIC (sem TSC-deadline) a ociosa ainda acorda a cada ~69 s, o que a contagem de 32 bits a 62,5 MHz alcança; no aarch64 e no TSC-deadline o prazo é de uma hora |
| Estado do gerador aleatório | `size_of::<Rng>()` (teste em `core/crypto/src/rng.rs`) | 40 bytes: chave de 32 e contador de 8 |
| Resíduo de chave na pilha | linha `key residue:` do boot (`random::stack_residue`), `make smoke ARCH=all PROFILE=dev\|release\|release-small` | 0 estados do ChaCha20 e 0 cópias da chave nas duas ISAs e nos três perfis; antes, de 0 a 2 e de 1 a 3 (tabela abaixo, no gerador) |
| Código de `core/crypto` | `.text` do crate sem LTO, com todas as funções públicas (`llvm-size`); no kernel, com LTO, só entra o que se usa | `release`: 8 202 (x86_64) e 7 866 (aarch64) bytes · `release-small`: 6 216 e 7 349 bytes |
| Vazão de ChaCha20, BLAKE2b e do gerador | `cargo run --release -p rustos-crypto --example bench`, no host (Apple M5, `rustc` 1.100.0), melhor de 15 rodadas, sem SIMD (o compilador não vetorizou: o resultado com `-vectorize-loops=false -vectorize-slp=false` é o mesmo) | Ver tabela abaixo. **Números do host**; os do alvo estão abaixo |
| Escalonador (`core/sched`) | `cargo test --release -p rustos-sched bench -- --ignored --nocapture --test-threads=1` (`core/sched/src/bench.rs`), no host | Host: `pick_next` do EEVDF em 41 a 136 ns com 8 a 512 tarefas; 96 bytes por tarefa ([SDD 14](14_escalonador.md)). No alvo (`bench`, 8 prontas): `pick_next` 34 a 35 ns e `tick` 2 ns no aarch64 HVF; 320 e 36 ns em TCG; 670 a 730 e 85 a 89 ns no x86_64 em TCG (tabela abaixo) |
| Boot de parede | `make smoke` | inclui firmware EDK2: ~2–5 s, dominado pelo firmware |

### Troca de contexto e jitter de acordar (fase 3)

Como se mede: `make screenshot ARCH=… PROFILE=release CMDS="bench;bench;bench"` e a linha
`switch:` do boot (as mesmas funções de `kernel/src/bench.rs`; SDD 15, "Medidas"). Perfil
`release`, QEMU 11.1.3 no macOS (Apple Silicon), 256 MiB. A aceleração decide o que o
número quer dizer (guidelines 05): só o HVF vale como latência; o TCG compara o RustOS
consigo mesmo. "Antes" é o kernel com o `RoundRobin` (commit `9aec89c`, o fim das tarefas
sem o EEVDF), "depois" é o `Composite` (FIFO e EEVDF) ligado, ambos medidos de novo na mesma
máquina; a faixa são as rodadas do `bench`.

| Máquina | | Troca, sem SIMD / com SIMD (ns) | Acordar, ociosa (média / máx., µs) | Acordar, uma ocupando a CPU (média / máx., µs) |
| --- | --- | --- | --- | --- |
| aarch64, **HVF**, `bench` | antes | 77–84 / 102–104 | 469–488 / 776–782 | 3,5–3,7 / 14–19 |
| | depois | 156–164 / 131–133 | 489–517 / 772–1 057 | 4,1–6,6 / 14–515 |
| aarch64, HVF, linha do boot | antes | 41 / 55 | 520 / 733 | — |
| | depois | 110 / 116 | 537 / 762 | — |
| aarch64, TCG, `bench` | antes | 390–392 / 452–456 | 1 192–1 227 / 2 117–12 845 | 1 137–1 278 / 2 031–17 277 |
| | depois | 838–850 / 949–966 | 1 223–1 241 / 2 057–2 095 | 526 / 1 058–1 084 |
| x86_64, TCG, `bench` | antes | 326–336 / 590–628 | 1 098–1 146 / 2 064–2 071 | 1 146–1 154 / 2 030–2 087 |
| | depois | 1 175–1 181 / 1 512–1 520 | 1 194–1 220 / 2 056–2 070 | 529–531 / 1 166–1 232 |

- **A troca com o EEVDF custa cerca de 2x a do rodízio no HVF** (77–84 para 156–164 no
  comando, 41 para 110 no boot) e de 2,5 a 3,5x em TCG. Cada ida e volta do ping-pong faz
  `wake`, `sleep`, `pick_next` e duas leituras do relógio por troca (uma em `unpark`, outra em `park`;
  a "três por ida e volta" que a dívida dizia era a conta velha, antes do `schedule_at`). Fora da política, medido à
  parte com um patch temporário que não entrou (HVF): `now_ns` 23 ns (`isb`, `mrs` e a
  conversão), `cycles` 15, `sleep` + `wake` 39, `next_deadline_ns` 2. Uma otimização que
  entrou: o `schedule` passou a receber o relógio que o `park` e o `yield` já leram, e o
  tempo de CPU da tarefa a ser contado em ns em vez de ciclos (uma leitura a menos por troca):
  141 para 99 ns na linha do boot. [DEBT-0042](../tech-debt/0042-troca-de-contexto-lenta-com-eevdf-e-relogio.md).
- **Política no alvo** (`pick_next` e `tick` com 8 tarefas prontas e relógio de mentira,
  linha `Scheduler policy` do `bench`): aarch64 HVF 34–35 e 2 ns; aarch64 TCG 319–320 e 36;
  x86_64 TCG 666–677 e 85–89. O do host (41 ns com 8 tarefas) e o do HVF concordam. O x86_64 é
  caro em TCG porque a divisão de 128 bits do `i128` é emulada
  ([DEBT-0041](../tech-debt/0041-escalonador-do-x86-sem-medida-fora-do-tcg.md)). `__divti3`
  e `__udivti3` de `compiler_builtins` linkam nas duas ISAs, em `release` e `release-small`.
- **Com SIMD custa mais 6 ns no aarch64 em HVF na linha do boot** (`stp`/`ldp` de 32
  registradores de 128 bits, mais FPSR e FPCR), e a ordem inverteu no comando (plain 160, SIMD 131): o primeiro
  ping-pong roda logo depois de a CPU ficar ociosa e o host ainda a mantém em frequência baixa.
  No x86_64 são ~330 ns em TCG (`xsaveopt` e `xrstor` interpretados: não é número de hardware).
- **O HVF mede o `wfi`, não o kernel, no jitter da ociosa.** Com a ociosa em `wfi` o
  thread do host da vCPU dorme até o prazo e acorda com ~0,5 ms de atraso. Trocando o `wfi`
  por um laço de espera, a média cai para 3,1 µs e a troca de contexto para 36 ns (com o
  rodízio; a CPU do host fica quente: sem `wfi` ela não baixa a frequência). Esse laço foi um
  patch temporário, fora do código; um boot só.
- **O TCG tem ~1,2 ms de média com a ociosa**, com o rodízio e com o EEVDF: o prazo passa pelo
  laço principal da QEMU e não pelo kernel (provável arredondamento do prazo a ms no `ppoll`
  do laço principal; não verificado no código da QEMU). Com uma tarefa ocupando a CPU, a média
  caiu de ~1,15 ms para ~0,53 ms depois de o tique de 100 Hz sair; não investiguei por quê.
- **Uma tarefa ocupando a CPU:** o `shell` (fatia de 200 µs) fura a proteção de fatia da
  tarefa de lote (700 µs) por `PREEMPT_SHORT`, e o acordar custa 4,1 µs, como com o rodízio.
  O pior caso com `N` ocupadas não foi medido. O 515 µs de máximo numa das rodadas do HVF é
  um valor isolado (as outras duas dão 14 e 15 µs).
- **Tique ocioso:** a linha `idle:` do boot, 2 IRQs do timer em 50 ms nas duas ISAs e em TCG
  e HVF. Eram 5 com o tique de 100 Hz.
- **Sem número de comparação.** O Linux (`lat_ctx`) e o Windows ainda não rodam na mesma
  máquina, e o `lat_ctx` passa o token por pipe; o ping-pong daqui o faz por
  `unpark` e `park`, então não é o mesmo teste. As fontes publicadas (1,2 a 2,2 µs em
  Haswell, pesquisa `scheduler.md`) são de outro hardware.

### Otimização da troca e do acordar 

Comandos, os mesmos antes e depois (`perf/scheduler-switch` contra a `main` `29882df`, `release`, QEMU 11.1.3, macOS, 4 CPUs):
`make smoke ARCH=aarch64 PROFILE=release` (linha `switch:` do boot) e, na mesma subida, o comando `bench` digitado na serial
9 s depois (scripts descartáveis; 5 boots por árvore, mínimo e mediana). Só o HVF vale como latência; o TCG só mostra a tendência.

| aarch64 HVF | antes (mín. / mediana) | depois (mín. / mediana) |
| --- | --- | --- |
| Troca, linha do boot (ns) | 68 / 72 | **41 / 41** |
| Troca com SIMD, linha do boot (ns) | 83 / 90 | 41 / 42 (um boot com 76) |
| Troca, comando `bench` (ns) | 73 / 132 | **42 / 42** (8 boots, um deu 101: o aquecimento de 300 ms nem sempre basta) |
| Troca com SIMD, comando `bench` (ns) | 91 / 125 | 43 / 45 |
| Acordar entre tarefas, mediana (`wake self-test:`) | 102 µs | **42 a 83 ns** |
| Acordar de `sleep_until`, ociosa (média, µs) | 472 a 514 | 504 a 562 (igual: é o `wfi` do HVF) |
| `pick_next` e `tick` da política, 8 prontas (ns) | 34 a 39 e 2 | 35 a 39 e 2 |
| x86_64 TCG, troca, linha do boot (ns), 3 boots | 1 398 / 1 519 | 1 294 / 1 307 |
| x86_64 TCG, troca, comando `bench` (ns), 3 boots | 1 595 / 1 603 | 1 276 / 1 282 |
| x86_64 TCG, acordar entre tarefas, mediana (`wake self-test:`) | não medido | 1 a 3 µs (a resolução do relógio do TCG); no `dev` com uma CPU a troca de contexto inteira sai a 30 µs em parte dos boots, antes (2 de 24) e depois (cerca de metade; SDD 15) |

O "antes" do comando varia de 73 a 158 ns porque ele chega depois de segundos ociosos e o hospedeiro mantém a vCPU em
frequência baixa (a árvore de antes não tem aquecimento); o "depois" mede com 300 ms de aquecimento (SDD 15, "Medidas"). O
ganho do comando mistura o aquecimento e a otimização; a linha do boot, que já vem quente nas duas árvores, é a comparação limpa.

**Decomposição do custo de uma troca** (código de antes, HVF; sonda descartável que mede cada peça em laço, 200 mil
voltas, melhor de 4; não está no código):

| Peça | ns |
| --- | --- |
| `now_ns`: `isb` + `mrs` (8,5) + atômicos + conversão por divisão de 128 bits (1,3) | 12,6 por leitura, duas por troca |
| `IrqLock` pegar e soltar (2,2 são o mascarar e desmascarar a IRQ) | 5,6, três por troca |
| Política por troca no ping-pong (`wake` + `sleep` + `pick_next`, duas tarefas) | 18 |
| `needs_resched`, `next_deadline_ns` | < 1 |
| Soma medida do ping-pong (o resto é `switch`, estado por tarefa, contadores) | 71 |

Passo a passo, o ping-pong na mediana de 9 medidas por boot (`context_switch_ns`, 20 000 voltas), aarch64 HVF:

| Passo | ns por troca | O que mudou |
| --- | --- | --- |
| Início | 71 a 75 | |
| Relógio do escalonador sem o `isb` | 48 | `platform::sched_clock_ns`: -23, mais que as duas leituras de 8,5 ns, porque o `isb` esvazia o pipeline no meio do caminho |
| Um lock da decisão à escolha; `wake_target` pinado | 46 | `park`, `sleep_until`, `yield_now` e `exit` passam o lock a `switch_from`; tarefa presa a uma CPU não varre as 8 cargas |
| Conversão por multiplicação e deslocamento | 42 | `TickScale`: -4 (isolado: `now_ns` 12,6 para 9,8 ns e `sched_clock_ns` 1,2 para 0,5) |
| `place` com divisão de 64 bits | (dentro do ruído) | no host, `sleep` + `wake` com 8 tarefas: 39 para 33 ns (`bench_round_robin_against_eevdf`) |

Tentado e **não** entrou, sem ganho medido: `recenter` sem divisão quando a soma já é o resto (host: 33 ns, igual), atalho
de `real_to_virtual` para o peso 1024 (HVF e host iguais), um `IrqLock` que não salva a máscara de IRQ já mascarada
(-1 a -2 ns, e pediria uma consulta nova à porta `Cpu`), e o `CNTVCTSS_EL0` (ECV zero no HVF, não se mede).

**O que restou da troca (41 ns):** a política ainda é 18 (o EEVDF faz `wake`, `sleep` e `pick_next` em árvore; o rodízio
faz o mesmo em ~6 no host), 11 dos dois locks, e o `switch`. [DEBT-0042](../tech-debt/0042-troca-de-contexto-lenta-com-eevdf-e-relogio.md)
fica aberta com o gatilho do `lat_ctx`.

**Acordar entre tarefas:** antes, `unpark` só levantava o `needs_resched` e a corrente seguia até a próxima IRQ (100 µs
medidos, mediana de três boots: 102 375, 102 458 e 102 417 ns; máximo de 107 a 172 µs). Agora troca no fim do `unpark`: 42 a 83 ns
de mediana, média de 54 a 72 ns e máximo de 125 a 250 ns nos boots medidos; uma amostra em umas 350 (4 em ~1 700) passa
de 350 µs (lag negativo do dorminhoco: o EEVDF não o deixa preemptar). O piso é o tique do contador (41,7 ns).
**O que não mudou e não é do escalonador:** o `wake:` do boot (`sleep_until` com a ociosa) continua em 0,5 ms (0,26 a 0,54 ms
conforme o estado do hospedeiro): é o `wfi` da vCPU no HVF (acima), e com uma CPU só e uma tarefa ocupando a outra, 3,7 µs.

### Criptografia do núcleo (host)

Saída do `bench` (o comando está na tabela acima). Não há comparação com outras
implementações: o número serve para o RustOS comparar consigo mesmo.

| Operação | Resultado |
| --- | --- |
| ChaCha20, `xor_keystream` de 4 MiB | ~910 MB/s |
| ChaCha20, um bloco de 64 bytes | ~58 ns |
| BLAKE2b-512 de 4 MiB | ~1 625 MB/s |
| BLAKE2b-256 com chave, 32 bytes de entrada (2 compressões) | ~189 ns |
| `Rng::next_u64` (um bloco por chamada) | ~67 ns |
| `Rng::fill_bytes` de 4 KiB | ~965 MB/s |
| `Rng::reseed` com 32 bytes | ~162 ns |

O BLAKE2b depende de a função G ser inlinada: sem `#[inline(always)]` ela cai de
1 584 para 507 MB/s (a exceção ao `clippy::inline_always` tem esse número como
motivo). O ChaCha20 não precisa.

### Criptografia do núcleo (alvo)

A linha `[rustos] random bench: chacha20 N MB/s, reseed N us` do boot. Mede
`Rng::fill_bytes` em 32 blocos de 4 KiB (128 KiB) e uma reposição inteira
(`random::replenish`: 64 bytes de hardware, se houver, 512 amostras de jitter e os
marcos do boot, com BLAKE2b). QEMU 11.1.3 no Apple M5, 3 rodadas por perfil, variação
dentro da faixa:

| Alvo | Perfil | ChaCha20 | Reposição | Vale? |
| --- | --- | --- | --- | --- |
| aarch64, HVF (sem RNDR: jitter) | `dev` | 421 a 470 MB/s | 37 a 40 us | sim |
| aarch64, HVF | `release` | 574 a 747 MB/s (636 a 694 antes do resíduo de chave) | 31 a 40 us | sim |
| aarch64, HVF | `release-small` | 317 MB/s | 40 a 42 us | sim |
| aarch64, TCG (`rndrrs`) | `dev` | 79 MB/s | 170 us | não: TCG |
| x86_64, TCG (`rdseed`) | `dev` | 39 a 40 MB/s | 278 a 291 us | não: TCG |

O `release` do alvo faz 70% do host (965 MB/s). A reposição de 37 us é quase toda o
jitter (inferência: o `reseed` de 32 bytes custa 162 ns no host).

**Zerar o resíduo de chave (custo e prova).** Mesmos comandos de antes e de depois (o "antes" é a `main`): o
`example bench` no host e a linha `random bench:` do boot (`next_u64` é a média de 20 000
pedidos de 8 bytes, com o gerador local de `benchmark`).

| Medida | Antes | Depois |
| --- | --- | --- |
| Host, `xor_keystream` de 4 MiB | 879 a 907 MB/s | 822 a 837 MB/s (-7%) |
| Host, `fill_bytes` de 4 KiB | 925 a 941 MB/s | 857 a 864 MB/s (-8%) |
| Host, bloco de 64 B | 55 a 58 ns | 56 a 60 ns |
| Host, `next_u64` | 67 a 68 ns | 69 a 72 ns (+3%) |
| Host, `reseed` de 32 B | 166 a 168 ns | 168 a 169 ns |
| aarch64 HVF `release`, `next_u64` (11 boots) | 70 a 92 ns | 85 a 104 ns |
| aarch64 HVF `release`, ChaCha20 (11 boots) | 684 a 897 MB/s | 574 a 747 MB/s |
| aarch64 HVF `release`, reposição | 29 a 38 us | 28 a 37 us |
| x86_64 TCG `release`, `next_u64` | 759 a 798 ns | 471 a 492 ns |
| x86_64 TCG `release`, ChaCha20 | 62 a 64 MB/s | 90 a 94 MB/s |

No host o custo é de 3% a 8% na vazão (o `Words` com `Drop` e o `permute` em `out`, em vez
de devolver 64 bytes por valor). No HVF as faixas se sobrepõem, mas a de depois desce (574 a 747 contra 684 a 897 MB/s): o custo é
de uns 10% a 15% e o ruído é do tamanho dele (inferência). O TCG do
x86_64 ficou mais rápido, o que é o código gerado e não vale como número de hardware.

Prova de que as cópias saíram: a linha `[rustos] key residue:` (`random::stack_residue`,
[SDD 09](09_seguranca.md)). Estados do ChaCha20 e cópias da chave achados nos 2 KiB abaixo do
quadro, antes e depois:

| Perfil | aarch64 antes | x86_64 antes | Depois (as duas ISAs) |
| --- | --- | --- | --- |
| `dev` | 1 e 2 | 0 e 1 | 0 e 0 |
| `release` | 1 e 1 | 1 e 1 | 0 e 0 |
| `release-small` | 2 e 3 | 1 e 1 | 0 e 0 |

**Limite de reposição: `RESEED_BYTES` = 4 MiB.** O `release` gera 4 MiB em 6,1 ms
(a 690 MB/s) e a reposição custa 37 us: 0,6% (0,3% no `release-small`). Com 1 MiB
seriam 2,4%. O limite é de volume entregue; reposição por tempo fica para a fase 3
([DEBT-0019](../tech-debt/0019-aleatorio-sem-fonte-de-hardware-fraco.md)). O x86_64
sem aceleração não tem número que valha:
[DEBT-0015](../tech-debt/0015-cripto-sem-medida-no-x86-acelerado.md).

### Alocadores de memória (fase 2, medido no host)

Comando: `cargo test --release -p rustos-mm bench -- --ignored --nocapture --test-threads=1`
(`core/mm/src/bench.rs`). Máquina: Apple M5 (aarch64), rustc 1.100.0, perfil `release`
(LTO, `opt-level = 3`), memória falsa em `Vec`, cache quente, região de 65 536 frames
(256 MiB). **Não é QEMU nem metal**: serve para comparar os alocadores entre si,
nunca com outro sistema (guidelines 05). Três rodadas, variação de 1 ns ou menos,
exceto o bitmap de uma rodada (7,7 e 8,9 ns, contra 2,0 e 4,0 nas outras duas).

| Operação (ns por par alocar+liberar, salvo "lote") | Bitmap | Buddy (a) bitmaps | Buddy (b) listas | Buddy (a) + cache de 64 |
| --- | --- | --- | --- | --- |
| 1 frame, memória virgem (cada par funde e parte 16 níveis) | 2 | 82 | 85 | — |
| 1 frame, anel de 4096 vivos | 4 | 21 | 14 | 1,7 |
| Lote: 32 768 alocações e 32 768 liberações, por operação | 1,7 | 9,8 | 13,6 | 13,1 |
| 2 MiB (512 frames), metade de baixo ocupada | 1 240 (varredura mais 512 liberações) | 33 | 37 | — |

- **O bitmap vence no frame solto** (um bit) e perde por duas ordens de grandeza no
  bloco contíguo, que ele acha por varredura. O buddy sozinho é 5x mais lento que o
  bitmap em 1 frame: o cache por CPU devolve o caso quente a 1,7 ns, e o lote (que o
  cache não acelera: 13 ns contra 10) é o custo do buddy.
- **A cascata de 16 níveis (82 ns) é o pior caso** de um par: liberar e pedir de novo
  um frame sobre memória toda livre parte e funde o bloco inteiro a cada vez. O cache
  por CPU absorve esse vaivém sem tocar o buddy.
- **(a) contra (b):** (b) ganha 7 ns no frame solto e perde 4 ns no lote e 4 ns no
  bloco grande, e escreve na memória livre; escolhemos (a) (SDD 12).

Heap (`KernelHeap`), 1000 objetos vivos, par alocar+liberar, mesma máquina:

| Pedido | Sem zerar | Zerando ao liberar | Custo de zerar |
| --- | --- | --- | --- |
| 16 B | 8 ns | 10 ns | +1 ns |
| 64 B | 8 ns | 10 ns | +2 ns |
| 1024 B | 8 ns | 17 ns | +9 ns |
| 4096 B (1 página, direto no buddy) | 27 ns | 52 ns | +25 ns |
| 8192 B (2 páginas) | 28 ns | 85 ns | +57 ns |
| 4096 B via cache de 64 frames | 1,7 ns | — | — |

Medida de novo depois do mapa de livres por objeto e da conferência `verify_used` antes
de zerar (3 rodadas de cada lado, mesma máquina, `cargo test --release -p rustos-mm
bench_heap -- --ignored`): o objeto pequeno foi de 7 para 8 ns (+1 ns do bit de livre) e
a página ou bloco direto, de 19 para 27 ns sem zerar (+7 ns: a conferência percorre o
buddy uma vez a mais), e de 46 para 52 ns zerando (+6 ns). É o preço de não zerar memória
já livre. O cache de 64 frames não muda (a conferência dele é nula).

Zerar custa cerca de 7 ns por KiB com o dado no cache: o piso. Com a memória fria
(página recém-devolvida por outro núcleo) o custo é o da banda da DRAM, e só vale
medir em metal ou com KVM/HVF.

**No alvo** (aarch64 sob HVF, `release`, par `alloc`+`dealloc` do `GlobalAlloc` com
1000 objetos vivos, 20 000 pares, relógio `platform::now_ns`, 3 rodadas):

| Pedido | Sem zerar | Zerando ao liberar | Custo de zerar |
| --- | --- | --- | --- |
| 16 B | 16 a 18 ns | 17 a 20 ns | +2 ns |
| 64 B | 16 a 18 ns | 18 a 20 ns | +2 a 3 ns |
| 1024 B | 15 a 18 ns | 53 a 57 ns | +38 a 40 ns |
| 4096 B (1 página) | 45 a 49 ns | 205 a 210 ns | +160 ns |
| 8192 B (2 páginas) | 35 a 38 ns | 323 a 330 ns | +290 ns |

São cerca de 39 ns por KiB, 5x o piso do host. A causa, no código do alvo (`llvm-objdump`
do `memset` do kernel em aarch64): o laço principal é um `str x, [x], #8`, uma gravação de 8
bytes por volta, sem `stp` nem SIMD. Para objetos até 64 bytes o custo é de 2 a 3 ns, e a
política fica ligada (SDD 09). A medida usou um patch temporário no autoteste do heap, que
não entrou no código. O x86_64 só roda em TCG aqui, sem número que valha.

#### Zerar pelo `arch/` (a dívida do `memset`, paga no aarch64)

`DirectMap::zero` manda ao `arch/` a faixa de 512 bytes ou mais, com início e tamanho
múltiplos de 64: `DC ZVA` em blocos inteiros e `STP` nas pontas e quando o `DCZID_EL0.DZP`
proíbe (SDD 13, "Zerar"). Comando: a linha `zero bench:` do boot (`heap::zero_benchmark`),
`make smoke ARCH=aarch64 PROFILE=release`. Cada coluna é o melhor de 3 rodadas de 4 000
repetições. aarch64 sob HVF, bloco do `DC ZVA` de 64 bytes, `DZP` limpo. O "par" é
`Vec::with_capacity(4096)` e o `drop`, com zerar ao liberar ligado.

**O "antes" não é um comando separado.** Vem do mesmo `zero_benchmark`, com o limiar de
512 bytes do `DirectMap::zero` elevado à mão para tudo ir ao `memset`, e foi tirado do
código: não há chave que o reproduza. O revisor do PR refez o A/B por conta própria e achou
ganho de 2,7 a 8x.

**Bimodal.** O 4 KiB cai em duas faixas, e a de cima aparece em cerca de um boot de cada
quatro: em 12 boots (11 com saída) ficou em 17,9 a 21 ns em oito, e em 42 a 55 ns em três; 1 KiB
foi de 12,2 a 16,4 ns (o revisor viu até 45,2). O `memset` tinha a mesma oscilação (124 a
131 ou 168 a 170). Pior caso medido do `DC ZVA`: 54,6 ns por 4 KiB, ainda 2,3x melhor que o
melhor do `memset`. A causa não foi investigada (inferência: o escalonamento dos núcleos do
Apple sob o HVF).

| ns | 1 KiB | 4 KiB quente | 4 KiB em 1 MiB | par alocar+liberar 4 KiB |
| --- | --- | --- | --- | --- |
| Antes (`memset`) | 36 a 47 (típico 37) | 124 a 169 (típico 130) | 123 a 167 | 186 a 243 (típico 187) |
| Depois (`DC ZVA`, `DCZID_EL0` lido a cada chamada) | 12 a 16 | 18 a 55 (típico 20) | 18 a 27 | 102 a 133 |
| Só laço de `STP` (`DZP` forçado, tirado depois) | 11 a 12 | 39 a 41 | 38 a 39 | — |

Ler o `DCZID_EL0` a cada chamada (é por CPU, como o Linux faz em cada `clear_page`) não
mudou o número: 4 KiB mínimo de 17,0 (lido uma vez) para 17,9 ns.

A página custa 6,5x menos no caso típico (130 para 20 ns, de 32 para 5 ns por KiB), e o par
de 4 KiB cai a metade. O laço de `STP` sozinho já leva a maior parte do ganho (gravar 16 bytes por
instrução em vez de 8); o `DC ZVA` soma cerca de 30% em cima dele, que é o caminho de
recuo e funciona. Abaixo de 512 bytes o `DC ZVA` perde para o `memset` (128 B: 6,9 contra
5,1 ns; 256 B: 12,0 contra 9,6 ns; 64 B empata, 2 a 3 ns), por isso o limiar. Os 1 KiB
batem no piso fixo de uns 10 ns do primeiro `DC ZVA`.

**x86_64, em TCG, sem mudança.** O `memset` do `compiler_builtins` do x86_64 sem a feature
`ermsb` já é `rep stosq` no corpo (`library/compiler-builtins/.../mem/x86_64.rs`,
`set_bytes`), a variante que o Linux escolhe sem ERMS (`clear_page_rep`,
`arch/x86/lib/clear_page_64.S`). O `rep stosb` com ERMS, que o Linux usa com ERMS
(`clear_page_erms`), mediu 8 772 a 9 207 ns por 4 KiB contra 1 089 a 1 156 ns do `stosq`,
8x pior, mas isso é o QEMU executar `rep stosb` um byte por vez, não a CPU: a medida em TCG
não decide nada. A QEMU anuncia ERMS com `-cpu max`, então escolher `stosb` por CPUID pioraria
o TCG sem número de hardware que o pague. Não entrou ([DEBT-0058](../tech-debt/0058-zerar-pagina-no-x86-sem-medida-em-hardware.md)).
Número do x86_64 hoje, o mesmo antes e depois: 1 KiB 290 a 310 ns, 4 KiB 1,1 µs, par de
4 KiB 2,0 µs.

### Escalonador (fase 3, medido no host; o alvo está acima)

Comando: `cargo test --release -p rustos-sched bench -- --ignored --nocapture
--test-threads=1` (`core/sched/src/bench.rs`). Máquina: Apple M5 (aarch64), rustc
1.100.0, perfil `release`, melhor de 7 rodadas de 100 000 chamadas, três execuções
(variação de 2 ns ou menos). **Não é QEMU nem metal**: serve para comparar as
políticas entre si, nunca com outro sistema (guidelines 05). Todas as tarefas são
CPU-bound, com nices de -3 a 3, e a capacidade é 512 nas duas políticas.

| Tarefas | Política | `tick` sem esgotar a fatia | `tick` que esgota | `pick_next` | `sleep` + `wake` |
| --- | --- | --- | --- | --- | --- |
| 8 | rodízio | 0,4 ns | 0,4 ns | 4 ns | 3 ns |
| 8 | EEVDF | 1,9 ns | 7,8 ns | 41 ns | 39 ns |
| 64 | rodízio | 0,5 ns | 0,5 ns | 4 ns | 17 ns |
| 64 | EEVDF | 1,9 ns | 7,8 ns | 108 ns | 141 ns |
| 512 | rodízio | 0,4 ns | 0,5 ns | 4 ns | 128 ns |
| 512 | EEVDF | 1,9 ns | 7,8 ns | 136 ns | 136 ns |

O `pick_next` é `(tick que esgota + pick_next) - tick que esgota`, num laço que
avança uma fatia de 10 ms por volta; o `sleep` + `wake` é de uma tarefa pronta que
não é a corrente. No rodízio, `sleep` e `remove` tiram do meio da fila (O(n)); no
EEVDF, o `pick_next` é dominado pelas duas atualizações do treap (devolver a corrente
e tirar a escolhida), não pela busca.

**Árvore contra varredura** (a escolha do elegível de menor prazo, com capacidade
igual ao número de tarefas): 2,3 / 2,8 / 5,8 ns pela descida do treap contra 11,8 /
90 / 815 ns pela varredura linear de 8 / 64 / 512 tarefas. A varredura serve de
oráculo nos testes e ficou como comparação.

**Otimização medida:** o `recenter` divide em `i64` quando a soma cabe (em vez de
`i128`): o `tick` que esgota a fatia caiu de 10,6 para 7,8 ns com 64 e 512 tarefas;
o `pick_next` não mudou, e o código cresceu cerca de 2 KiB no `release` do x86_64
(13 441 para 15 590 bytes, inlining).

**Memória** (`size_of`, `bench_memory_per_task`): `Eevdf<512>` = 49 232 bytes, ou 96
por tarefa (`Entity` 56, nó da árvore 32, tabela de ids 8); `RoundRobin<512>` = 8 752
(17 por tarefa); `Fifo<512>` = 7 600 (14 por tarefa e as 99 listas).

**Código** (`Composite<8, 64>` e `RoundRobin<64>` instanciados numa biblioteca de
prova fora do repositório, `.text` do objeto com LTO, `llvm-size`): x86_64 15 590
bytes no `release` e 8 700 no `release-small`; aarch64 15 228 e 10 148. A divisão em
128 bits vem de `compiler_builtins`, fora dessa conta. No kernel, só entra o que se
usa.

**Latência de wakeup na simulação** ([SDD 14](14_escalonador.md)), 4 tarefas sempre
prontas e uma tarefa que roda 0,2 ms e dorme 5 ms, timer de um tiro: espera média de
2,5 µs com fatia de 0,1 ms e 863 µs com fatia de 6 ms; no pior caso espera algumas
fatias, não o ciclo todo.

### SMP: troca entre núcleos, IPI, escala e contenção (fase 3, 2ª onda)

Como se mede: `make screenshot ARCH=… PROFILE=release CMDS="bench;bench"` (a PNG mostra o último), as linhas
`switch:`, `scale:` e `alloc storm:` do boot (`make smoke PROFILE=release`, esforço curto) e `RUSTOS_SMP=1|2|4`. Mesmo código
em `kernel/src/bench.rs` ([SDD 15](15_tarefas_e_preempcao.md), "Medidas"). QEMU 11.1.3 no macOS (Apple Silicon), 256 MiB, `-smp 4`.
**Aceleração:** o aarch64 roda em **HVF** (hardware: vale como latência); o x86_64 só roda em **TCG**, e o aarch64 também em TCG com
`RUSTOS_ACCEL=tcg`. Neste QEMU os vCPUs do TCG rodam em threads do hospedeiro, também no x86_64 (observado, não conferido no código do QEMU: 10 threads e cerca de 280% de CPU durante o `bench`).
**O hospedeiro estava carregado por outro processo pesado (um outro QEMU) durante parte das rodadas**: a escala e a contenção do TCG oscilam
entre rodadas; abaixo vão as rodadas típicas e, onde a oscilação foi grande, o intervalo.

| Medida | aarch64 HVF | aarch64 TCG | x86_64 TCG |
| --- | --- | --- | --- |
| Troca na mesma CPU (ping-pong, ns por troca) | 64–73 (boot), 141 (`bench`); com `-smp 1`, 73–80; kernel da 1ª onda hoje: 81 e 109 | 1 039 | 1 494–1 617 |
| Troca com SIMD (idem) | 83–89 (boot), 133 (`bench`) | 1 065 | 1 860–1 991 |
| Troca **entre CPUs** (cada passagem de vez é um acordar remoto por IPI) | 5 400–6 040 | 4 675 | 4 394–4 747 |
| IPI de ida e volta | 10 300–10 700 | 9 080 | 8 029–9 074 |
| Escala, 8 tarefas de 10 ms (ms): 1 / 2 / 4 CPUs | 76,3 / 41,7 (1,8x) / 22,0 (3,4x) | 14,5 / 8,5 (1,7x) / 4,7 (3,1x), esforço curto de 2 ms | 1,0x / 1,5x / 1,4x a 1,5x no `bench`; 1,0x / 1,8x / 3,3x no esforço curto, quando o hospedeiro estava livre |
| `pick_next` e `tick` da política (ns) | 35–48 e 2 | 419 e 43 | 644–679 e 86–87 |

Leitura:

- **A troca na mesma CPU não piorou com a fila por CPU**: no HVF o `-smp 1` do kernel novo (73–80 ns) e o kernel da 1ª onda medido hoje (81–109 ns) ficam na mesma faixa; contadores de uma CPU só deixaram de usar
  `lock xadd` (`sync::bump`), o que pesa no TCG. A faixa de 99 a 164 ns do SDD da 1ª onda era de outra rodada.
- **Acordar em outra CPU custa de 40 a 80 vezes uma troca local no HVF** (5,4 a 6 µs contra 70 a 140 ns): é o IPI, que no HVF é uma escrita em `ICC_SGI1R_EL1` que sai para o hipervisor, e a saída de `wfi`. O IPI de ida e volta (cerca de 10,5 µs) é o dobro dessa troca remota.
  Não há número publicado confiável para comparar: a pesquisa não achou fonte primária do custo de IPI ([pesquisa de escalonador](#números-das-pesquisas-e-o-grau-de-verificação)).
- **A escala é de 3,4x em 4 núcleos no HVF** (8 tarefas de 10 ms: 76 ms em 1 CPU, 22 ms em 4; o ideal é 19 ms): o resto é spawn, o `wait` e os IPIs. O TCG do aarch64 dá 3,1x.
  No x86_64 em TCG a escala mede o hospedeiro, não o kernel: com o hospedeiro livre deu 3,3x (esforço curto), com ele carregado ficou em 1,4x a 1,5x; o `scale:` do boot do x86_64 oscila entre uma coisa e outra ([DEBT-0041](../tech-debt/0041-escalonador-do-x86-sem-medida-fora-do-tcg.md)).
- **Contenção da memória, 4 CPUs alocando e devolvendo 4 frames por volta** (aarch64 HVF, 20 000 voltas por CPU): **17,9 ms e 16 tentativas que acharam o lock preso** com o buddy sob lock, **0,8 ms e nenhuma** com o cache por CPU: 22 vezes mais rápido, e é por isso que o
  cache está ligado ([SDD 12](12_memoria_fisica_e_heap.md)). No aarch64 TCG (2 000 voltas): 21,3 ms (158 516 disputas) contra 2,1 ms. No x86_64 TCG com 2 vCPUs: 14,4 ms (30 448) contra 1,6 ms; com 1 vCPU, 5,5 contra 1,6 ms; com 4 vCPUs, **não se
    reproduziu**: o esforço curto deu 299 ms contra 127 ms, mas o `bench` completo deu 237 a 291 ms contra 1 266 a 1 306 ms (cache mais lento, sem disputa nenhuma). Medido de novo com o hospedeiro livre, o cache ganhou de 10 a 13 vezes (215 contra 22 ms, 218 contra 16 ms), tanto no TCG padrão quanto com `tcg-multi`: a anomalia era o hospedeiro carregado ([DEBT-0041](../tech-debt/0041-escalonador-do-x86-sem-medida-fora-do-tcg.md)).
- **O heap tem um lock só e disputa**: a tempestade de objetos de 64 bytes deu 3,3 ms (17 tentativas presas) no HVF, e o `bench` fecha com contenção acumulada de milhares de tentativas no lock do heap ([DEBT-0054](../tech-debt/0054-heap-com-lock-global.md)). Fila de execução: 277 a 279 tentativas em tudo o que o `bench` faz; console: 0.
- **Migração:** o autoteste de SMP (8 tarefas, 40 ms) faz de 117 a 150 migrações em 565 a 631 voltas no x86_64 (TCG) e de 1 145 a 1 648 em 5 551 a 6 217 no aarch64 (HVF), 16 vezes sem falha.
- **Memória do SMP:** ver a linha "Custo por CPU" acima. O cache de frames segura até 124 KiB por CPU fora do buddy.

### Para onde foi o tamanho do kernel

A fase 0.5 (tela, terminal, fonte e shell) levou o kernel `release` de 13 784 para
100 541 bytes no x86_64, e o `release-small` de 11 104 para 61 725 bytes. Os
maiores símbolos do x86_64 `release` (`llvm-nm --size-sort`):

| Símbolo | Bytes | O que é |
| --- | --- | --- |
| `system::run_shell` | 22 162 | laço do shell com `Shell::feed` e os comandos inlinados pelo `opt-level = 3` |
| `Terminal::apply` | 7 405 | despacho de todas as sequências CSI/ESC |
| `kmain` | 5 260 | boot com `Screen::new` e o relatório inlinados |
| `Screen::write` | 3 366 | tradução de `\n` e desenho das linhas sujas |
| `font::DIRECT` | 3 056 | glifos de U+0020–007E e U+00A0–00FF |
| `commands::colors`, `commands::fetch` | 2 819 + 2 811 | os dois comandos que mais formatam texto |

Rolar a tela suja a grade inteira: cada linha nova com o cursor no fim repinta
todas as células (1280x800: cerca de 4 MB de escrita no framebuffer). O desenho
acontece uma vez por mensagem e por tecla, não por pedaço de texto, mas ainda não
há comparação com o que já está na tela nem rolagem por cópia.

Também entram tabelas Unicode da `core` (`grapheme_extend`, `cn_planes`, ~2 KiB)
puxadas por formatação `{:?}`. Nenhuma otimização de tamanho foi feita ainda: a
regra do guidelines 05 pede o número de antes e depois, e este é o "antes". O
tempo de desenho de uma tela cheia ainda não é medido: o kernel não tem relógio
até a fase 1.

Para comparação: o Linux guarda um `struct page` de 64 bytes por página de 4 KiB,
cerca de 1,6% da RAM; o trabalho de *memdescs* mira 8 bytes, cerca de 0,2%
([Wilcox, 2022](https://lore.kernel.org/all/YvZmDfSFMydiulzw@casper.infradead.org/T/);
[LWN](https://lwn.net/Articles/1015320)). O bitmap é mais barato porque só guarda
livre/ocupado; quando houver páginas compartilhadas e copy-on-write, o kernel vai
precisar de um descritor por frame, e a meta é ficar na faixa de 8–32 bytes. O buddy com bitmaps
custa 0,25 byte por frame, sem descritor: o descritor entra com a contagem de
referências (fase 4), não com o buddy.

### Linha de base de memória ociosa (Linux de distro com raiz mínima)

Passo 1 do método do [SDD 08](08_roadmap.md#memória-ociosa-o-alvo-vem-de-uma-medida),
feito em 2026-11-17. O Linux é só instrumento de medida: não entra no produto. **O "Linux mínimo"
desta base é um kernel `virt` de distro** (de 13 a 14 MiB de código e `bss` de 9,3 MiB no aarch64) com
uma raiz mínima, não um kernel enxuto: o `tinyconfig` não foi medido (próximo passo, abaixo).

**O que foi medido**

- **Linux:** Alpine 3.24.4, kernel `6.18.60-0-virt` (`#1-Alpine SMP PREEMPT_DYNAMIC`),
  do `alpine-netboot-3.24.4-<isa>.tar.gz` de `dl-cdn.alpinelinux.org/alpine/v3.24/releases/`
  (a `v3.25` do mirror ainda não tem `releases/`, e a `latest-releases.yaml` aponta a
  3.24.4). SHA-256 dos tarballs conferido com o do `latest-releases.yaml` do Alpine:
  `1502e776…951a8` (x86_64) e `9d10f909…9745e` (aarch64). Só `vmlinuz-virt` e
  `initramfs-virt` foram usados. O Alpine não publica soma por arquivo do netboot, então
  os arquivos extraídos não têm soma própria conferida.
- **Como sobe sem rede:** `-append "console=ttyS0 rdinit=/bin/sh"` (`ttyAMA0` no aarch64). O
  `init` do Alpine não roda: não há `busybox --install`, módulo, `modloop` nem
  `mdev`. É o kernel de distro mais um `sh` ocioso, o mais próximo de "kernel + init mínimo".
- **Mesmas flags do RustOS** (`xtask/src/arch.rs`, `qemu_machine_args`), QEMU 11.1.3, host
  Apple M5 com macOS 26.4: `-machine q35 -cpu max -m 256M -accel tcg` (x86_64) e
  `-machine virt,gic-version=3 -cpu max -m 256M -accel hvf` (aarch64), `-nic none`,
  `-no-reboot`, `-display none -serial stdio -monitor none`, mais `-smp N`. O RustOS não passa
  `-smp` hoje (1 CPU); mediu-se com 1 e com 4.
- **Pelo EDK2, como o RustOS:** `-bios edk2-aarch64-code.fd` (aarch64) e `-drive
  if=pflash,format=raw,readonly=on,file=edk2-x86_64-code.fd` (x86_64), com `-kernel` e `-initrd`:
  o EDK2 carrega o Linux pelo stub EFI. O disco FAT e o `ramfb` do RustOS ficaram de fora:
  o Linux não os usa.
- **Leitura:** 10 s depois da linha `Run /bin/sh as init process` no console, digitada na
  serial: `/bin/busybox mount -t proc proc /proc` e `/bin/busybox cat /proc/meminfo`.
  Três boots frios (um processo da QEMU novo cada), mediana por campo.
- **Duas raízes em RAM.** O initramfs vira a raiz em `tmpfs` e conta como usado
  (`Shmem` não entra no `MemAvailable`):
  - **publicada:** o `initramfs-virt` sem mudança. Descomprimida tem 30 175 676 bytes (28,8 MiB,
    x86_64) e 25 713 532 (24,5 MiB, aarch64), dos quais 66% e 59% são `lib/modules` e o resto, bibliotecas do instalador (`libcrypto`, `libapk`); `Shmem` lê 29,1 e 24,9 MiB.
  - **mínima:** o mesmo kernel com um initramfs que montei só com o `busybox` e o carregador
    musl tirados do `initramfs-virt` (`Shmem` 1,4 e 1,6 MiB). Para o alvo, é a base a usar, porque a
    publicada mede o pacote de módulos de um instalador, não o ocioso.

**`MemAvailable` e usado, pelo EDK2 (256 MiB de RAM)**

| ISA | Raiz em RAM | `-smp` | `MemAvailable` nos 3 boots (MiB) | Mediana (MiB) | Usado: 256 − mediana (MiB) |
| --- | --- | --- | --- | --- | --- |
| x86_64 (TCG) | mínima | 1 | 183,8 / 183,7 / 183,9 | **183,8** | **72,2** |
| x86_64 (TCG) | mínima | 4 | 183,9 / 185,1 / 186,0 | **185,1** | **70,9** |
| x86_64 (TCG) | Alpine como publicado | 1 | 151,6 / 152,0 / 152,4 | **152,0** | **104,0** |
| x86_64 (TCG) | Alpine como publicado | 4 | 157,1 / 156,4 / 156,8 | **156,8** | **99,2** |
| aarch64 (HVF) | mínima | 1 | 194,2 / 194,3 / 194,3 | **194,3** | **61,7** |
| aarch64 (HVF) | mínima | 4 | 186,0 / 185,1 / 186,8 | **186,0** | **70,0** |
| aarch64 (HVF) | Alpine como publicado | 1 | 163,4 / 170,5 / 170,4 | **170,4** | **85,6** |
| aarch64 (HVF) | Alpine como publicado | 4 | 163,0 / 165,6 / 166,1 | **165,6** | **90,4** |

**Campos de `/proc/meminfo`, medianas em MiB (EDK2)**

| ISA | Raiz | `-smp` | MemTotal | MemFree | MemAvailable | Buffers | Cached | Shmem | KernelStack | PageTables | Slab |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| x86_64 | mínima | 1 | 210,0 | 186,7 | 183,8 | 0,0 | 1,4 | 1,4 | 1,1 | 0,4 | 8,1 |
| x86_64 | mínima | 4 | 209,2 | 187,9 | 185,1 | 0,0 | 1,4 | 1,4 | 1,5 | 0,5 | 10,9 |
| x86_64 | publicada | 1 | 210,0 | 154,7 | 152,0 | 0,0 | 29,1 | 29,1 | 1,1 | 0,4 | 9,2 |
| x86_64 | publicada | 4 | 209,2 | 159,3 | 156,8 | 0,0 | 29,1 | 29,1 | 1,5 | 0,5 | 11,8 |
| aarch64 | mínima | 1 | 210,2 | 196,8 | 194,3 | 0,0 | 1,6 | 1,6 | 0,8 | 0,2 | 7,0 |
| aarch64 | mínima | 4 | 209,1 | 188,4 | 186,0 | 0,0 | 1,6 | 1,6 | 1,1 | 0,2 | 9,4 |
| aarch64 | publicada | 1 | 210,2 | 172,8 | 170,4 | 0,0 | 24,9 | 24,9 | 0,8 | 0,2 | 7,6 |
| aarch64 | publicada | 4 | 209,1 | 167,8 | 165,6 | 0,0 | 24,9 | 24,9 | 1,1 | 0,2 | 9,9 |

**Sem o EDK2** (`-kernel` direto, SeaBIOS no x86_64 e sem firmware próprio no aarch64), com a raiz
publicada. O firmware reserva memória diferente da do caminho UEFI: a `MemTotal` sobe 4,5 MiB (aarch64) e
5,8 MiB (x86_64), e o disponível sobe 2,9 e 10,5 MiB contra a tabela do EDK2. A comparação com o
RustOS usa a do EDK2, que é o caminho dele. A memória que o firmware não entrega conta dos dois lados:
no Linux x86_64 pelo EDK2 são 6 392 KiB fora da linha `Memory:` (262 144 menos 255 752), perto dos 6 624 KiB
do RustOS (262 144 menos 255 520, `memory:`); no aarch64 ela está nos `reserved` do Linux (58 092 KiB pelo
EDK2 e 53 500 direto) e nos 8 996 KiB do RustOS.

| ISA | `-smp` | `MemAvailable` nos 3 boots (MiB) | Mediana (MiB) | Usado (MiB) |
| --- | --- | --- | --- | --- |
| x86_64 (TCG) | 1 | 162,6 / 162,5 / 162,5 | 162,5 | 93,5 |
| x86_64 (TCG) | 4 | 162,2 / 162,6 / 161,4 | 162,2 | 93,8 |
| aarch64 (HVF) | 1 | 173,3 / 175,2 / 173,3 | 173,3 | 82,7 |
| aarch64 (HVF) | 4 | 167,7 / 166,9 / 168,4 | 167,7 | 88,3 |

**RustOS** (`main` em `24be32a`, `release`, 1 CPU, `make smoke` e `make screenshot ... CMDS="mem"`):

| ISA | `memory:` | `reclaimed:` | `frames:` | `mem`: livre | Disponível (livre + cache devolvível) | Usado: 256 − disponível |
| --- | --- | --- | --- | --- | --- | --- |
| x86_64 (TCG) | 255 520 KiB usable | 43 332 KiB | 63 809 livres de 65 268 | 249,1 MiB (usado 380 KiB, reservado 2,0 MiB) | **249,1 MiB** | **6,9 MiB** |
| aarch64 (HVF) | 253 148 KiB usable | 30 316 KiB | 63 231 livres de 65 536 | 246,9 MiB (usado 320 KiB, reservado 12 KiB) | **246,9 MiB** | **9,1 MiB** |

O cache de página do RustOS não existe ainda, então o disponível é o livre do buddy. Dos
6,9 e 9,1 MiB "usados", só 0,4 e 0,3 MiB são memória em uso dentro do mapa usável; o resto
(6,5 e 8,8 MiB) é o que o firmware e o bootloader não entregam como RAM. O mesmo buraco existe do lado
do Linux (ver acima), e a imagem do kernel do RustOS está nesse resto, não nos 0,4 MiB "em uso".

**O que cada lado tem**

- **Linux:** kernel `virt` de distro (a linha `Memory:` do `dmesg` soma code, rwdata, rodata, `init` e
  `bss`: 30 MiB no x86_64 e 34 no aarch64; o `init`, de 2 a 3 MiB, é liberado depois), `struct page` por frame, caches do slab (7 a 12 MiB), um BusyBox e a raiz em `tmpfs`.
  Sem módulos carregados, sem rede (`-nic none`), sem disco montado.
- **RustOS:** kernel carregado de 230,7 KiB (x86_64) e 198,9 KiB (aarch64) (`make size PROFILE=release`: 236 320 e 203 729 bytes), shell embutido no kernel, console na
  tela e na serial, buddy, heap, escalonador, tarefas. Sem espaço de usuário, sem cache de
  página, sem drivers de disco ou de rede.

**O que a diferença não prova**

- Que o RustOS final vai gastar de 10 a 15% do que a base gasta (hoje 6,9 contra 72,2 MiB no x86_64 e 9,1
  contra 61,7 no aarch64). Hoje ele não tem nada que custe memória por processo, por arquivo ou por
  dispositivo; o Linux também não tem na base, mas paga o custo fixo do kernel de distro inteiro.
- **Que o RustOS ganha de um Linux enxuto.** A base é um kernel de distro; contra um `tinyconfig` o
  Linux gastaria menos e a razão do RustOS subiria. Por isso o alvo "≤ metade" é uma barra frouxa.
- Que a ordem se mantém com espaço de usuário, cache de página e drivers (fases 4 a 6). Cada
  um acrescenta memória; a medida só vale de novo com o BusyBox rodando em cima do RustOS.
- Que o shell embutido equivale a um `sh` de usuário: o do Linux é um processo com pilha, `mm`
  e arquivos; o do RustOS é uma função do kernel.

**Alvo proposto (hipótese, a confirmar na fase 5)**

> Com 256 MiB de RAM e o mesmo `-smp` da base, o RustOS com kernel e BusyBox ocioso usa
> **no máximo metade do que a base (Linux de distro, raiz mínima) usa**, mediana de 3 boots: **usado ≤ 36 MiB**
> (disponível **≥ 220 MiB**) no x86_64 e **usado ≤ 30,8 MiB** (disponível **≥ 225,2 MiB**) no aarch64.

Metade da base deixa de folga de 29 MiB (x86_64) e 22 MiB (aarch64) sobre o que o RustOS usa
hoje, para espaço de usuário, cache e drivers. Se a fase 5 estourar isso, a hipótese está
errada e muda com o número na mão, pelo caminho de Decisões. Ao confirmar: repetir a base com o `-smp`
que o RustOS tiver e com a mesma ferramenta, porque no aarch64 o Linux gasta de 5 a 8 MiB a mais com 4 CPUs
(`Slab`, pilhas e dados por CPU); no x86_64 não houve aumento.

**Próximo passo:** medir um Linux enxuto (`tinyconfig` mais BusyBox e init) nas mesmas condições, para a
base deixar de ser a de uma distro; se ele ficar bem abaixo, o alvo acima aperta.

**Limitações**

- **Um host.** x86_64 em TCG (emulação de CPU) e aarch64 em HVF: o x86_64 mede a memória do guest,
  que não depende do acelerador, mas não é medida de metal.
- **Dispersão entre os 3 boots** (máximo menos mínimo do `MemAvailable`): de 0,1 a 2,1 MiB na maioria
  das linhas (2,1 no x86_64 mínima `-smp 4`, 1,7 no aarch64 mínima `-smp 4`, 1,9 e 1,5 no aarch64 direto,
  1,2 no x86_64 direto `-smp 4`), 3,1 MiB no aarch64 publicada `-smp 4` e 7,1 MiB no aarch64 publicada
  `-smp 1`, onde o 1.º boot ficou abaixo dos outros dois (163,4 contra 170,5 e 170,4 MiB) e a mediana o
  descarta. Três amostras é o que o método pede, mas não dá dispersão confiável.
- **A raiz mínima é montagem minha**, não artefato do Alpine; a publicada é a do Alpine e infla o usado.
- **Pelo EDK2, o x86_64 com a raiz publicada gasta menos com 4 CPUs do que com 1** (de 104,0 para
  99,2 MiB, 4,8 MiB, maior que a dispersão): não explicado. Com a raiz mínima a diferença (1,3 MiB) está
  dentro do ruído.
- **`MemAvailable` não é a mesma conta do buddy livre:** o kernel desconta dele de 2,5 a 3 MiB
  (`MemFree − MemAvailable`) que o RustOS não desconta. O `free` do BusyBox não foi usado (SDD 08).
- **Sem disco, ramfb nem rede no Linux.** O RustOS sobe com o disco FAT e, no aarch64, com o `ramfb`;
  o Linux subiu sem nenhum dos dois.

## Método de comparação com Linux e Windows

1. **Mesma máquina, mesmo firmware, versões anotadas.** Firmware e ACPI mudam o
   tempo de boot; cada rodada registra host, QEMU, firmware e versão do sistema
   ([SUSE, KVM Forum 2022](https://gitlab.com/qemu-project/kvm-forum/-/raw/main/_attachments/2022/QEMU%20%26%20KVM%20Automated%20Performance%20Benchmarking%20%40%20SUSE-1.pdf)).
2. **QEMU para função, metal para latência.** Sob TCG o mesmo boot de Linux levou
   ~8,25 s contra ~1,22 s com KVM
   ([Santilli](https://cirosantilli.com/linux-kernel-module-cheat/benchmark-linux-kernel-boot.html)):
   número de latência só vale com aceleração de hardware ou em metal.
3. **Boot medido até a primeira instrução de usuário**, não até um shell.
4. **Microbenchmarks com o mesmo binário de teste** nos três sistemas, quando houver
   ABI Linux (fase 5): syscall nula, troca de contexto, page fault, IPC. No Windows,
   o equivalente funcional mais próximo, com o método escrito ao lado do número.
5. **Memória ociosa**: RAM livre depois do boot sobre RAM total, com a mesma RAM. O
   alvo vem de uma linha de base de Linux medida na mesma QEMU, não de chute:
   o método está no [SDD 08](08_roadmap.md#memória-ociosa-o-alvo-vem-de-uma-medida) e a
   medida em [Linha de base de memória ociosa](#linha-de-base-de-memória-ociosa-linux-de-distro-com-raiz-mínima).

## O que a pesquisa aponta, por subsistema

Levantamento de novembro de 2026. Três graus de confiança, na coluna *Fonte*:

| Marca | Quer dizer |
| --- | --- |
| (sem marca) | Fonte primária lida nesta pesquisa: documentação ou código do projeto, spec, README. Vale só para as linhas desta rodada; as de levantamentos anteriores dizem *não lida nesta rodada* |
| **secundária** | Blog, notícia, lista de patches ou resumo de busca: serve de ordem de grandeza, não de número a copiar |
| **não verificado** | Não confirmado nesta pesquisa: lido de memória, de trecho de busca ou não achado. Não vira decisão sem ler a fonte primária |

| Subsistema | Técnica | Fonte | Fase |
| --- | --- | --- | --- |
| Escalonador | Fila por CPU, sem lock global; política de prazo/lag (EEVDF, que substituiu o CFS no Linux 6.6) atrás de um trait | [kernel.org EEVDF](https://docs.kernel.org/scheduler/sched-eevdf.html), [LWN](https://lwn.net/Articles/925371) | 2–3 |
| Escalonador | Política plugável, no espírito do `sched_ext` (filas de despacho por CPU) (*não lida nesta rodada*) | [kernel.org sched_ext](https://docs.kernel.org/scheduler/sched-ext.html) | depois de 3 |
| Memória física | Cache de páginas por CPU, reabastecido em lote, sobre um buddy allocator; buddy é necessário para blocos contíguos e páginas grandes (*não lida nesta rodada*) | [linux-mm](https://www.linux-mm.org/PageAllocation) | 2–3 |
| Memória física | Descritor por frame de 8–32 bytes (memdescs/folios) (*não lida nesta rodada*) | [FOSDEM 2025](https://fosdem.org/2025/events/attachments/fosdem-2025-4860-shrinking-memmap/slides/238482/Wilcox-Me_GEqPExc.pdf) | 4 |
| Memória virtual | PCID/ASID com contador de geração por CPU; pular TLB shootdown em CPU em modo *lazy* | `arch/x86/mm/tlb.c` do Linux (espelho, não verificado no oficial) | 3 |
| Syscall | Submissão em lote por fila compartilhada (FlexSC, io_uring) em vez de uma trap por chamada (*não lida nesta rodada*) | [FlexSC](https://www.usenix.org/legacy/events/atc11/tech/final_files/Soares.pdf) | 4 |
| IPC | Caminho rápido curto, no modelo do seL4; o número entra quando for medido aqui | [seL4 performance](https://sel4.systems/About/Performance/) (página do projeto; nenhum número conferido) | 4 |
| Mitigações | Ligar só as mitigações de Spectre/Meltdown que a CPU precisa, e relatar quais estão ligadas (*não lida nesta rodada*) | [Linux admin guide](https://docs.kernel.org/admin-guide/hw-vuln/spectre.html) | 4 |
| Interrupções | MSI-X e troca para polling acima de uma taxa, no modelo NAPI (*não lida nesta rodada*) | [kernel.org NAPI](https://docs.kernel.org/networking/napi.html) | 10 |
| Escalonador | EEVDF em detalhe: lag é o que a tarefa deveria ter recebido menos o que recebeu; elegível se `lag >= 0`; entre as elegíveis, a de menor deadline virtual (elegibilidade mais slice). `avg_vruntime` sai das somas ponderadas da fila, sem varrer. `PLACE_LAG` guarda o lag no sono, `DELAY_DEQUEUE` deixa quem dorme com lag negativo pagar na fila, `RUN_TO_PARITY` adia a preempção do acordar. Slice base de 0,70 ms, escalado por `1 + ilog(ncpus)`. Interativa = slice curto e lote = slice longo é mapeamento nosso; a fonte não usa esses termos. **latency-nice**: a interface não existe no `master` lido; o status de merge é **não verificado** | [kernel.org EEVDF](https://docs.kernel.org/scheduler/sched-eevdf.html) (conceitual); `fair.c`, `features.h` e `uapi/linux/sched/types.h` do `master`; [LWN 925371](https://lwn.net/Articles/925371) (**secundária**) | 3 |
| Escalonador | Tempo real: SCHED_FIFO e RR de prioridade 1 a 99; teto de 95% de banda, que vale para todos os processos de tempo real e de prazo do sistema (950000/1000000, `sched(7)`). Detalhe do pull de CPU ociosa, de `select_task_rq` e custo de IPI: **não verificado** (`fair.c` lido só até 100 KB) | [sched(7)](https://man7.org/linux/man-pages/man7/sched.7.html), [sched-deadline](https://docs.kernel.org/scheduler/sched-deadline.html), [sched-domains](https://docs.kernel.org/scheduler/sched-domains.html) | 3 |
| Tempo | Timer de um tiro com expiração absoluta, TSC-deadline no x86_64; NO_HZ_IDLE antes de NO_HZ_FULL; timer slack zero para quem precisa de precisão. Jitter do acordar: **não verificado**, medir aqui | [hrtimers](https://docs.kernel.org/timers/hrtimers.html), [no_hz](https://docs.kernel.org/timers/no_hz.html), [PR_SET_TIMERSLACK](https://man7.org/linux/man-pages/man2/PR_SET_TIMERSLACK.2const.html); TSC-deadline pelo commit 279f1461 num espelho de terceiro (**secundária**) | 3 |
| Troca de contexto | Estado FPU **eager**, salvo com `XSAVEOPT` (formato padrão, pula o que não mudou) no x86_64. O lazy só ganha para tarefa que não usa FPU, e custa um #NM caro (1156 ciclos) e o vazamento do LazyFP ([SDD 09](09_seguranca.md)). Jogo usa SIMD em quase toda troca: o caso bom do lazy não ocorre. Custo de `XSAVEOPT` com AVX-512 ou AMX: **não verificado**, nenhuma medida publicada; medir aqui | [Lutomirski, 2011](https://lkml.iu.edu/1107.3/00141.html) (`xsave`, `xsaveopt`, `xrstor` num Sandy Bridge); [SDM espelhado: `xsaveopt`](https://www.felixcloutier.com/x86/xsaveopt) (**secundária**: reproduz o SDM, que não foi lido); [Cyberus, LazyFP](https://www.cyberus-technology.de/post/intel-lazyfp-vulnerability-exploiting-lazy-fpu-state-switching); [commit ca6938a1 (outubro de 2016, janela da 4.9, "x86/fpu: Hard-disable lazy FPU mode"; a mensagem diz que o eager já era o padrão desde a 4.6)](https://github.com/torvalds/linux/commit/ca6938a1cd8a1c5e861a99b67f84ac166fc2b9e7); [`xstate.c`](https://raw.githubusercontent.com/torvalds/linux/master/arch/x86/kernel/fpu/xstate.c) | 3 |
| Troca de contexto | Tamanho da área de XSAVE lido uma vez do CPUID 0xD e guardado: a consulta custa de 348 a 1 700 ciclos conforme a sub-folha e a CPU (tabela abaixo) | Commit do Linux num espelho de terceiro (**secundária**, [ctrliq](https://github.com/ctrliq/kernel-src-tree/commit/1201f226c863b7da739f7420ddba818cedf372fc)) | 3 |
| Troca de contexto | aarch64: FPSIMD eager por simplicidade e custo, não por segurança: o LazyFP é da Intel (V0–V31, FPSR, FPCR: ~520 B, derivado); SVE e SME por trap de primeiro uso, VL padrão pequeno. Bits de `CPACR_EL1` e valores dos campos: **não verificado** (só espelho de terceiro) | [Linux SVE](https://docs.kernel.org/arch/arm64/sve.html), [`fpsimd.c`](https://raw.githubusercontent.com/torvalds/linux/master/arch/arm64/kernel/fpsimd.c); `CPACR_EL1` em [espelho](https://df.lth.se/~getz/ARM/SysReg/AArch64-cpacr_el1.html) (**secundária**) | 3, 4 |
| Processos | Estado grande por pedido: AMX fora do `XCR0` inicial, liberado por processo (`ARCH_REQ_XCOMP_PERM`); AVX-512 e PKRU ligados sozinhos | [Linux xstate](https://docs.kernel.org/arch/x86/xstate.html) | 4 |
| Descarte de página | LRU de gerações, com refault como sinal de desempate (MGLRU). O ganho vem de descartar melhor, não de caber mais. Custo de CPU num SO pequeno: **não verificado** | [kernel.org MGLRU](https://docs.kernel.org/mm/multigen_lru.html); números na tabela abaixo (**secundária**) | 6 |
| Pressão de memória | PSI (`some`, `full`, `avg10/60/300`, gatilho por `poll()`) desde o início; daemon de pressão em espaço de usuário, que age por limiar de PSI antes do OOM do kernel. `memory.high` (desacelera, não mata) antes de `memory.max` | [PSI](https://docs.kernel.org/accounting/psi.html), [lmkd do Android](https://source.android.com/docs/core/perf/lmkd), [cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html) (lido até o caractere 100 000 de 143 188) | 9 |
| Compressão de memória | LZ4 no lugar de zstd para páginas: 2,11x a 770/4950 MB/s contra 2,90x a 510/1570 MB/s (zstd -1). Só sem swap em disco e com CPU ociosa. Razão em páginas reais: **não verificado** (o zram só assume 2:1) | READMEs de [lz4](https://github.com/lz4/lz4) e [zstd](https://github.com/facebook/zstd) (lzbench, i7-9700K); [zram](https://docs.kernel.org/admin-guide/blockdev/zram.html) | 9 |
| Compartilhamento | KSM: só páginas anônimas com `MADV_MERGEABLE`, desligado por padrão; `rmap_item` de 64 B por página varrida; passa de ~64 itens por página fundida e o lucro some. Sem número de economia real. Fica para depois da fase 9 | [KSM](https://docs.kernel.org/admin-guide/mm/ksm.html) | depois de 9 |
| Gráfico | Buffer do cliente em memória compartilhada ou dma-buf, sem cópia; `damage_buffer` por região, nunca a tela inteira; reuso do buffer quando o compositor já guarda cópia. Bytes por janela e custo do damage: **não verificado**, medir aqui (`largura * altura * 4 * 2` é só a conta da dupla bufferização) | [Wayland](https://wayland.freedesktop.org/docs/html/ch03.html) (**secundária**: lido por resumo; o protocolo não foi lido), [dma-buf](https://www.kernel.org/doc/html/latest/driver-api/dma-buf.html) | 11 |
| Jogos | Escada de seis marcos, de DOOM por software a Mesa com virgl ou Venus ([SDD 08](08_roadmap.md#jogos-a-escada)) | [QEMU virtio-gpu](https://www.qemu.org/docs/master/system/devices/virtio/virtio-gpu.html), [Mesa Venus](https://docs.mesa3d.org/drivers/venus.html); Venus no HVF: **secundária** | 5–13 |
| Segurança × custo | Núcleo pequeno com `unsafe`, resto seguro; o Asterinas relata desempenho próximo ao Linux com ~14% de TCB (números do próprio projeto) (*não lida nesta rodada*) | [ATC '25](https://www.usenix.org/conference/atc25/presentation/peng-yuke) | já adotado |

### Números das pesquisas e o grau de verificação

Números de outros sistemas, de hardware e data diferentes do RustOS: **não se
comparam entre si nem com o que o RustOS mede**. Servem para dizer o que esperar.

| Número | Contexto | Fonte | Grau |
| --- | --- | --- | --- |
| `xsave` 24 ns, `xsaveopt` 16 ns, `xrstor` 40 ns; 128 ns para `xsave` + `clflush` + `xrstor` com `ymm` não zero | Sandy Bridge i7-2600, 2011, cache quente | [Lutomirski](https://lkml.iu.edu/1107.3/00141.html) | Primária, lida; CPU antiga |
| Escrita em CR0 208 ciclos; falta de TS (#NM) 1156 ciclos | Molnar, 2015, CPU não anotada | [lista do kernel](https://lists.openwall.net/linux-kernel/2015/05/29/725) | Primária, lida |
| Tarefa sem FPU 5% a 10% mais rápida na troca com lazy | Molnar, 2015. A mesma discussão nota que a troca entre tarefa lazy e tarefa eager é bem mais lenta | mesma discussão | Primária, lida |
| CPUID 0xD sub-folha 2: 348 ciclos (Skylake-SP) e 1138 (Emerald Rapids); sub-folha 13: 1700 | Medida no commit do kernel | [ctrliq](https://github.com/ctrliq/kernel-src-tree/commit/1201f226c863b7da739f7420ddba818cedf372fc) | **Secundária**: espelho de terceiro |
| Par `kernel_fpu_begin`/`end`: 160 ns (xmm), 340 (ymm), 360 (zmm); syscall nua ~70 ns | Eric Biggers, 2024 | URL não capturada | **Não verificado** |
| `xsaveopt` ~100 ciclos mesmo sem nada modificado | Lutomirski, 2018 | URL não capturada | **Não verificado** |
| Troca de contexto: 1,4 us (lmbench, Linux 2.2, PowerPC de 400 MHz) e 1,8 us (Celeron de 500 MHz), 2000; 1,2 a 1,5 us pinado e ~2,2 us sem pino (Haswell i7-4771, 2018) | `lat_ctx` e pipe ping-pong. Em VM e em kernel recente: não achado | [lkml, 2000](https://lkml.indiana.edu/0001.2/1182.html); [Bendersky, 2018](https://eli.thegreenplace.net/2018/measuring-context-switching-and-memory-overheads-for-linux-threads/) | Lista do kernel lida; o blog é **secundária** |
| Desligar as páginas globais, efeito do PTI, custa menos de 1% | `pti.rst` do Linux 5.5. Não é o custo do KPTI inteiro (troca de CR3, TLB) | [espelho](https://kernelsources.org/source/xref/linux/Documentation/arch/x86/pti.rst?a=true) | **Secundária**: espelho |
| LZ4 2,112x, 770 / 4950 MB/s; zstd -1 2,896x, 510 / 1570 MB/s; LZ4 HC -9 2,717x, 40 / 4800 MB/s | lzbench 2.4.1, i7-9700K, Silesia, uma thread | [README do lz4](https://github.com/lz4/lz4) | Primária, lida. O README do zstd dá 2,101x e 675/3850 MB/s para o LZ4 1.10 na mesma máquina |
| ~1 us por página de 4 KiB com LZ4 contra ~2,5 us com zstd -1 | Conta a partir dos MB/s acima | — | **Conta nossa**, não de fonte |
| `rmap_item` do KSM: 32 B (32 bits) e 64 B (64 bits) por página varrida | Documentação | [KSM](https://docs.kernel.org/admin-guide/mm/ksm.html) | Primária, lida |
| MGLRU v11, em produção: CPU do kswapd -40%, mortes por pouca memória -85% (p75), abertura de app -18% (p50) | Sem versão de kernel no texto | [LWN 895658](https://lwn.net/Articles/895658/) | **Secundária**: resumo da página; conferir o texto antes de citar |
| MGLRU v1 (Chrome OS, 5.11): CPU do kswapd -51%, PSI `full` -52%. Android: 18% menos mortes, 16% menos cold starts (simulação). Chrome OS em campo: 96% menos descartes de aba, 59% menos OOM | Cover letter da série | [lore](https://lore.kernel.org/all/YFAoMS67UeqA3IhW@google.com/) | **Secundária**: trecho de busca, texto integral não lido |
| Windows 10 comprime páginas pouco usadas "tipicamente em 40%"; macOS 10.9 "mais de 50%" | Notícias | [Petri](https://petri.com/memory-compression-in-windows-10-threshold-2/); [AppleInsider](https://appleinsider.com/articles/13/06/12/compressed-memory-in-os-x-109-mavericks-aims-to-free-ram-extend-battery-life) | **Secundária**. A documentação primária da Microsoft não foi achada |
| Memória ociosa em VM: OpenWrt 78 MB, Alpine 162 MB, Tiny Core 175 MB (com o desktop ligado); Alpine ISO "47 a 59 MB" (lista antiga); Redox 0.3.3 "~480 MB" | Benchmarks de blog, listas e notícia | [ComputingForGeeks](https://computingforgeeks.com/lightweight-linux-distros-iot-old-hardware/), [lista do Alpine](https://lists.alpinelinux.org/~alpine/users/%3C65b3fc8bc0887e1fb24248e678f5599b%40riseup.net%3E), [Phoronix](https://www.phoronix.com/news/Redox-OS-0.3.3) | **Secundária**, ordem de grandeza. Linux `tinyconfig` e Windows 11 ocioso: **não verificado**. **Não é alvo** ([SDD 08](08_roadmap.md#memória-ociosa-o-alvo-vem-de-uma-medida)) |
| Virgl com GL 4.3 (4.6 com blob), Venus com Vulkan 1.1 | Documentação do QEMU e da Mesa | [QEMU](https://www.qemu.org/docs/master/system/devices/virtio/virtio-gpu.html), [Mesa](https://docs.mesa3d.org/drivers/venus.html) | Primária, lida |
| Venus no macOS/HVF fora do QEMU upstream: RFC de dez/2025, v3 de mar/2026 | Lista de patches | patchew | **Secundária**: não conferida no repositório do QEMU |
| Redox roda SDL e QuakeSpasm pelo Orbital | Cookbook do Redox | cookbook | **Secundária** |

## Princípios que viram regra

As regras verificáveis que saem daqui estão no
[guidelines 05](../guidelines/05_recursos_e_desempenho.md): sem crate externo, sem
alocação no caminho quente, estrutura de tamanho fixo no núcleo, e nenhuma
otimização sem número de antes e depois.
