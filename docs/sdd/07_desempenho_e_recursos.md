# SDD 07 — Desempenho e recursos

> **O que este capítulo responde:** o que já se mede, como se vai comparar com Linux
> e Windows, e quais técnicas a pesquisa aponta para cada subsistema.

## O que já se mede

| Métrica | Como | Valor hoje (QEMU, 256 MiB) |
| --- | --- | --- |
| Kernel carregado | `make size PROFILE=release` / `release-small` | x86_64: 235 680 / 147 492 bytes · aarch64: 202 941 / 132 025 bytes (fase 3 com o EEVDF no kernel: +24 584 / +12 136 (x86_64) e +20 520 / +11 824 (aarch64) sobre o mesmo kernel com o rodízio; `.data` do `SCHED`, 3 968 bytes; o resto é a política, o `i128` do EEVDF e a medida da política no `bench`). Antes, a fase 3 só com o rodízio, tarefas, preempção, estado de SIMD, timer de um tiro e `ps` e `bench`: 211 096 / 135 356 e 182 421 / 120 201, medidos de novo no commit anterior (+26 763 / +13 856 e +29 440 / +16 956 sobre a fase 2, que a linha a seguir descreve). Antes, a fase 2 depois da revisão do PR 5: 184 093 / 121 404 e 153 101 / 103 129 (`.data.rel.ro` só leitura, caminhada única em `core/mm`, bit de livre no slab). Antes da revisão: 183 669 / 120 004 e 155 889 / 102 021. Antes: com buddy, heap e gerador, 166 544 / 106 095 e 136 292 / 88 364; antes do buddy, 133 304 / 88 735 e 105 448 / 71 508; na fase 0.5, 100 541 / 61 725 e 86 500 / 59 952. O PIE do KASLR custa 3 289 / 5 581 (x86_64) e 5 361 / 6 265 (aarch64), de 1,8% a 6,6% ([SDD 13](13_memoria_virtual.md)) |
| Memória livre depois do boot (o "pronto quando" da fase 2) | `frames:` do boot (QEMU 256 MiB, `dev`) e `mem` | x86_64: 63 811 frames (249,3 MiB, 97,4% da RAM), antes 52 321 (204,4 MiB) · aarch64: 63 237 (247,0 MiB, 96,5%), antes 54 991 (214,8 MiB). Devolvido: 46 432 e 33 424 KiB |
| Tabelas de página do kernel | `vm:` do boot | 60 a 64 KiB (x86_64) e 44 a 48 KiB (aarch64) com 2 MiB no mapa direto; 548 e 536 KiB só com 4 KiB. Varia com o deslocamento sorteado do mapa direto |
| Grade do terminal | 8 bytes por célula (`size_of::<Cell>()`), em frames contíguos | x86_64 1280x800: 158x47 células, 59 408 bytes (15 frames) · aarch64 1024x768: 126x45, 45 360 bytes (12 frames) |
| Fonte própria | tabelas em `.rodata` (teste em `core/gfx/src/font`) | 3 884 bytes para 237 glifos 8x16 |
| Shell | `size_of::<Shell<256, 32>>()`, na pilha do `kmain` | 8 880 bytes |
| Metadado de memória física | `BuddyAllocator::words_needed` (SDD 12): um bitmap por ordem com resumo hierárquico; é o `metadata` da linha `frames:` do boot e o `frame meta` do `mem` | 16 704 bytes para 65 094 frames (x86_64) e 16 504 para 64 396 (aarch64): 2,05 bits por frame, 0,008% da RAM; 2,03 bits por frame em 4 GiB ou mais. O bitmap de antes, que saiu do kernel: 8 144 e 8 056 bytes |
| Heap do kernel | `heap::self_test` e a linha `heap` do `mem` | Metadado de 64 bytes por slab, dentro da página, com o mapa de livres de 1 bit por objeto (SDD 12). Zerar ao liberar ligado: custo abaixo |
| Aleatório no kernel | linha `random bench:` do boot (`random::benchmark`): 32 blocos de 4 KiB com `Rng::fill_bytes` e uma reposição completa, com o relógio da fase 1 | aarch64 sob HVF, `release`: 636 a 694 MB/s e 36 a 38 us por reposição (tabela abaixo) |
| Boot até o kernel | linha `boot timeline` (pedido *bootloader performance* do Limine) | Limine: 52 ms (aarch64, HVF) / 350 ms (x86_64, TCG) |
| Pilha de falhas (aarch64) | `Memory::map_stack`: 4 páginas mapeadas e uma de guarda, por CPU; é o `SP_EL0` do EL1h (SDD 15) | 16 384 bytes de RAM, mais as tabelas de página que faltarem |
| Custo por tarefa | SDD 15, "Custo por tarefa": `SCHED`, `Context`, pilha e área de FPU/SIMD | ~250 bytes de tabela e política (3 968 de `SCHED` com 16 slots); 8 (x86_64) ou 104 (aarch64) de `Context`; 32 768 de pilha (mais as tabelas de página); 832 (x86_64 da QEMU, `xsaveopt` com AVX) ou 528 (aarch64) de FPU/SIMD, só da tarefa que usa |
| Troca de contexto | `bench` e a linha `switch:`: ping-pong de `unpark`/`park` entre duas tarefas, ns por troca, aquecimento e melhor de 3 | aarch64 HVF `release`, com o EEVDF: 99 a 116 ns no boot e 131 a 164 no comando; com o rodízio eram 42 e 77 a 84 (tabela abaixo); x86_64 e aarch64 em TCG só para comparar o RustOS consigo mesmo |
| Jitter de acordar | `bench`: `agora − pedido` depois de `sleep_until`, 200 amostras | aarch64 HVF, com o EEVDF: 0,49 a 0,52 ms (ociosa em `wfi`) e 4,1 µs (CPU ocupada) de média; igual ao do rodízio (tabela abaixo) |
| Timer (aarch64) | linha `timer:` do boot; `CNTV` com `CVAL` absoluto, 100 Hz só até `task::init`; depois não há tique periódico (linha `idle:`) | 50 tiques em 497 ms (HVF) e 488 ms (TCG) no autoteste; ociosa com o EEVDF: 2 IRQs do timer em 50 ms, contra 5 a 100 Hz (HVF, TCG e x86_64 iguais) |
| Estado do gerador aleatório | `size_of::<Rng>()` (teste em `core/crypto/src/rng.rs`) | 40 bytes: chave de 32 e contador de 8 |
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
  `wake`, `sleep`, `pick_next` e três leituras do relógio. Fora da política, medido à
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
| aarch64, HVF | `release` | 636 a 694 MB/s | 36 a 38 us | sim |
| aarch64, HVF | `release-small` | 317 MB/s | 40 a 42 us | sim |
| aarch64, TCG (`rndrrs`) | `dev` | 79 MB/s | 170 us | não: TCG |
| x86_64, TCG (`rdseed`) | `dev` | 39 a 40 MB/s | 278 a 291 us | não: TCG |

O `release` do alvo faz 70% do host (965 MB/s). A reposição de 37 us é quase toda o
jitter (inferência: o `reseed` de 32 bytes custa 162 ns no host).

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

São cerca de 39 ns por KiB, 5x o piso do host: o `memset` do alvo é o suspeito
([DEBT-0020](../tech-debt/0020-zerar-pagina-pelo-memset-generico.md)). Para objetos
até 64 bytes o custo é de 2 a 3 ns, e a política fica ligada (SDD 09). A medida usou um
patch temporário no autoteste do heap, que não entrou no código. O x86_64 só roda em TCG aqui, sem número que valha.

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
   alvo vem de uma linha de base de Linux mínimo medida na mesma QEMU, não de chute:
   o método está no [SDD 08](08_roadmap.md#memória-ociosa-o-alvo-vem-de-uma-medida).

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
