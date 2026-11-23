# SDD 07 — Desempenho e recursos

> **O que este capítulo responde:** o que já se mede, como se vai comparar com Linux
> e Windows, e quais técnicas a pesquisa aponta para cada subsistema.

## O que já se mede

| Métrica | Como | Valor hoje (QEMU, 256 MiB) |
| --- | --- | --- |
| Kernel carregado | `make size PROFILE=release` / `release-small` | **Hoje, com a fatia 4a** (entrada de syscall, exceções de usuário, cópia, processo e a limpeza da pilha; o modo de teste leva também o programa de usuário e o autoteste): x86_64: 363 867 / **192 231** bytes · aarch64: 307 593 / **175 069** bytes. **Antes da 4a** (sobre a `main` `9f31a09`, mesmo comando): x86_64: 346 448 / 183 556 (+17 419 / +8 675, +5,0% e +4,7%) · aarch64: 292 309 / 166 665 (+15 284 / +8 404, +5,2% e +5,0%). Antes: (sobre a `main` `b495b0c`, que tinha 346 760 / 210 692 bytes no x86_64 e 292 633 / 189 501 no aarch64): x86_64: 346 448 / 183 556 bytes · aarch64: 292 309 / 166 665 bytes. A `release-small` deixou de levar o modo de teste (−12,9% e −12,1%, [abaixo](#imagem-enxuta-sem-o-modo-de-teste)); a `release` o leva (`selftest`). Antes disso: x86_64: 325 616 / 192 900 bytes · aarch64: 272 641 / 174 457 bytes (a otimização da troca e do acordar custou +7 704 / +2 472 e +5 164 / +2 368 sobre os 317 912 / 190 428 e 267 477 / 172 089 da `main` `5fe47b4`: o ponto de preempção de `unpark`, `TickScale`, a medida e o autoteste de acordar entre tarefas e o aquecimento do `bench`; o `.bss` ficou igual, as amostras vão para a pilha do acordador e a ordenação é por inserção). Antes dela: x86_64: 317 912 / 190 428 bytes · aarch64: 267 477 / 172 089 bytes (log sem endereço do kernel e pânico visível desde a entrada: +9 400 / +4 896 e +7 752 / +5 220 sobre a `main` `29882df`, mesmo comando: o filtro de endereço, a linha `kaslr:`, os vetores mínimos, a serial precoce e o `early::print`; sem `.bss` novo, porque as tabelas do `TTBR0_EL1` do aarch64 vivem na pilha do Limine). Antes: x86_64: 308 512 / 185 532 bytes · aarch64: 259 725 / 166 869 bytes (o SMP da fase 3 custou +47 024 / +26 512 e +38 148 / +25 088 sobre os 261 488 / 159 020 e 221 577 / 141 781 que seguem, sem ele: fila por CPU, migração, IPI, pânico, cache de frames e o `bench` de SMP; o `.bss` do SMP é de poucos KiB, porque a política de cada CPU vem do heap). Antes do SMP: x86_64: 261 488 / 159 020 bytes · aarch64: 221 577 / 141 781 bytes (sobre 250 448 / 155 388 e 213 693 / 138 573 da `main` `85ae25e`, mesmo comando: +11 040 / +3 632 e +7 884 / +3 208. É o zerar por `DC ZVA`, o autoteste de faixas e a linha `zero bench:`, mais o resíduo de chave (`Words`, o autoteste `key residue:` com a sonda e o `next_u64` do `random bench:`); boa parte é a formatação das duas linhas de medida). Antes, fase 3 com o EEVDF no kernel: +24 584 / +12 136 (x86_64) e +20 520 / +11 824 (aarch64) sobre o mesmo kernel com o rodízio; `.data` do `SCHED`, 3 968 bytes; o resto é a política, o `i128` do EEVDF e a medida da política no `bench`). Antes, a fase 3 só com o rodízio, tarefas, preempção, estado de SIMD, timer de um tiro e `ps` e `bench`: 211 096 / 135 356 e 182 421 / 120 201, medidos de novo no commit anterior (+26 763 / +13 856 e +29 440 / +16 956 sobre a fase 2, que a linha a seguir descreve). Antes, a fase 2 depois da revisão do PR 5: 184 093 / 121 404 e 153 101 / 103 129 (`.data.rel.ro` só leitura, caminhada única em `core/mm`, bit de livre no slab). Antes da revisão: 183 669 / 120 004 e 155 889 / 102 021. Antes: com buddy, heap e gerador, 166 544 / 106 095 e 136 292 / 88 364; antes do buddy, 133 304 / 88 735 e 105 448 / 71 508; na fase 0.5, 100 541 / 61 725 e 86 500 / 59 952. O PIE do KASLR custa 3 289 / 5 581 (x86_64) e 5 361 / 6 265 (aarch64), de 1,8% a 6,6% ([SDD 13](13_memoria_virtual.md)) |
| Memória livre depois do boot (o "pronto quando" da fase 2) | `frames:` do boot (QEMU 256 MiB, `dev`) e `mem` | x86_64: 63 811 frames (249,3 MiB, 97,4% da RAM), antes 52 321 (204,4 MiB) · aarch64: 63 237 (247,0 MiB, 96,5%), antes 54 991 (214,8 MiB). Devolvido: 46 432 e 33 424 KiB |
| Tabelas de página do kernel | `vm:` do boot | 60 a 64 KiB (x86_64) e 44 a 48 KiB (aarch64) com 2 MiB no mapa direto; 548 e 536 KiB só com 4 KiB. Varia com o deslocamento sorteado do mapa direto |
| Grade do terminal | 8 bytes por célula (`size_of::<Cell>()`), em frames contíguos | x86_64 1280x800: 158x47 células, 59 408 bytes (15 frames) · aarch64 1024x768: 126x45, 45 360 bytes (12 frames) |
| Tela alternativa do terminal | Uma segunda grade, do mesmo tamanho da principal, entregue pelo kernel (`set_alternate`, [SDD 16](16_terminal.md)) | 8 bytes por célula: x86_64 59 408 bytes (15 frames) · aarch64 45 360 bytes (12 frames) |
| Histórico de rolagem do terminal | Anel de linhas inteiras (`set_history`): `cols * 8` bytes por linha, `HISTORY_LINES = 256` em `kernel/src/screen.rs`; gravar uma linha é um `copy_from_slice` de `cols` células. Só com teclado: o aarch64 da `virt` não aloca (`history 0 lines` no boot, [DEBT-0050](../tech-debt/0050-historico-sem-rolagem-no-aarch64.md)) | x86_64 1 264 bytes por linha, 323 584 no total (79 frames) · aarch64 1 008 por linha, 258 048 que ficam sem alocar enquanto não houver teclado |
| CPU do histórico de rolagem | Medido pela revisão do PR: `release`, host, melhor de 7, com 256 linhas de histórico contra nenhum | 20 mil linhas de 158 colunas rolando: +5% a +10%, cerca de 15 ns por linha rolada; nas outras cargas, dentro do ruído |
| Redesenho da tela cheia | `console::flush` com tempo de `now_ns` em volta, `colors` três vezes, `release`, maior valor visto (medida descartável, não está no código) | x86_64 (QEMU TCG): 27,7 ms · aarch64 (QEMU com HVF): 1,5 ms. É o custo de Shift+PageUp e de qualquer linha nova com a tela cheia; roda com IRQ mascarada ([DEBT-0051](../tech-debt/0051-redesenho-da-tela-com-irq-mascarada.md)). Número de TCG só compara o RustOS consigo mesmo |
| Custo da tela alternativa, do histórico e das caixas no boot | `frames:` do boot (QEMU 256 MiB, `dev`) e `make size`, na `main` e com a mudança | Frames livres, x86_64: 63 801 → 63 704 (97 frames, 388 KiB, com o histórico) · aarch64: 63 227 → 63 211 (16 frames, 64 KiB, só a grade alternativa). Kernel `release` / `release-small`: x86_64 236 320 → 251 200 (+14 880) / 147 732 → 155 388 (+7 656) · aarch64 203 729 → 213 693 (+9 964) / 132 297 → 138 569 (+6 272). Inclui `alt`, `lines` e os 9 glifos novos |
| Fonte própria | tabelas em `.rodata` (teste em `core/gfx/src/font`) | 4 046 bytes para 246 glifos 8x16 (3 884 para 237 antes dos 9 glifos do DEC Special Graphics: ◆ ≤ ≥ π ≠ ⎺ ⎻ ⎼ ⎽) |
| Shell | `size_of::<Shell<256, 32>>()`, num `static` (`system::SHELL`, `.bss`) desde que construí-lo por valor custava 19 KiB de pilha em `release` | 8 880 bytes |
| Metadado de memória física | `BuddyAllocator::words_needed` (SDD 12): um bitmap por ordem com resumo hierárquico; é o `metadata` da linha `frames:` do boot e o `frame meta` do `mem` | 16 704 bytes para 65 094 frames (x86_64) e 16 504 para 64 396 (aarch64): 2,05 bits por frame, 0,008% da RAM; 2,03 bits por frame em 4 GiB ou mais. O bitmap de antes, que saiu do kernel: 8 144 e 8 056 bytes |
| Heap do kernel | `heap::self_test` e a linha `heap` do `mem` | Metadado de 64 bytes por slab, dentro da página, com o mapa de livres de 1 bit por objeto (SDD 12). Zerar ao liberar ligado: custo abaixo |
| Zerar página | linha `zero bench:` do boot (`heap::zero_benchmark`), `make smoke ARCH=… PROFILE=release` | aarch64 HVF: 4 KiB em 20 a 38 ns (antes 124 a 169), 1 KiB em 13 a 16 (antes 36 a 47); x86_64 TCG: 1,1 µs por 4 KiB, sem mudança (tabela no heap, acima) |
| Aleatório no kernel | linha `random bench:` do boot (`random::benchmark`): 32 blocos de 4 KiB com `Rng::fill_bytes` e uma reposição completa, com o relógio da fase 1 | aarch64 sob HVF, `release`: 636 a 694 MB/s e 36 a 38 us por reposição (tabela abaixo) |
| Boot até o kernel | linha `boot timeline` (pedido *bootloader performance* do Limine) | Limine: 52 ms (aarch64, HVF) / 350 ms (x86_64, TCG) |
| Pilha de falhas (aarch64) | `Memory::map_stack`: 4 páginas mapeadas e uma de guarda, por CPU; é o `SP_EL0` do EL1h (SDD 15) | 16 384 bytes de RAM, mais as tabelas de página que faltarem |
| Custo por CPU | SDD 15, "Custo por tarefa e por CPU": `PerCpu`, a política no heap, a pilha e as pilhas de exceção | ~500 bytes de `PerCpu` (`.bss`, por CPU possível) mais 3 680 de política no heap por CPU que entra no escalonador; núcleo secundário: 16 KiB de pilha (eram 32) mais 48 KiB de IST (x86_64) ou 16 KiB de pilha de falhas (aarch64); cache de frames: 256 bytes |
| Custo por tarefa | SDD 15, "Custo por tarefa e por CPU": `Slot`, `Context`, pilha e área de FPU/SIMD | ~140 bytes de `Slot` e 96 de entidade na política justa; 8 (x86_64) ou 104 (aarch64) de `Context`; 16 384 de pilha (eram 32 768; mais as tabelas de página); 832 (x86_64 da QEMU, `xsaveopt` com AVX) ou 528 (aarch64) de FPU/SIMD, só da tarefa que usa |
| Pico de pilha das threads do kernel | coluna `stack` do `ps` e linha `stacks:` do boot (`task::log_stacks`), `dev` e `release`, nas duas ISAs (SDD 15, "Pico de pilha") | Tarefa: 2,2 a 2,5 KiB; ociosa: 0,8 a 1,2 KiB (6,7 KiB `dev`, 3,9 `release` e 8,8 `release-small` no núcleo secundário, com o init); shell: 12,9 a 16,0 KiB no boot e até 17,5 KiB depois do `bench` (eram 42,6 em `release` enquanto o shell era construído por valor na pilha). Pilha da thread: **16 KiB (eram 32)**, 176 KiB a menos no `mem` (720,0 para 544,0 KiB usados, aarch64 `release`, 4 CPUs, depois do `bench`) |
| Custo de pintar a pilha | `Stack::paint` e `repaint` com o contador em volta (medida descartável, não está no código), `dev` e `release` | aarch64 HVF: 1 a 2 us por pilha de 16 KiB no primeiro `spawn` do slot e 0,5 a 0,7 us por `spawn` que reusa (varredura exata); x86_64 TCG: 8 a 11 us por reuso, só compara o RustOS consigo mesmo |
| Custo de pintar as pilhas no boot normal | `make smoke-fast`, 5 boots em `release`, linha `boot time:` na `main` (`f65c815`) e com a pintura | aarch64 HVF: 2 ms nos dois, 5 de 5; x86_64 TCG: 24 a 25 ms antes e 24 ms depois (4, 4, 4, 5, 5 antes e 4, 4, 4, 4, 4 de `memory`). O número é de 1 ms de resolução; o custo estimado, de pintar as 5 pilhas do boot normal, é de poucas dezenas de us no HVF e de 0,1 a 0,3 ms no TCG (a medida de 13 pilhas deu 0,2 a 0,25 ms), abaixo da resolução: não se pintou de forma preguiçosa |
| Conferência de lock preso na troca | linha `switch:` do boot, 5 boots em aarch64 HVF `release`, antes e depois de trocar o `debug_assert_eq!(locks_held(), …)` por uma conferência que vale em `release` | 41 a 43 ns antes (média 42,0), 42 a 44 depois (média 42,6): um load e um compare por troca, **cerca de 1 ns** (a revisão mediu 42,0 para 43,2 em 6 pares intercalados); com a recusa de IRQ em `release` também, 41 a 42 (41,4). O contador tem resolução de 41,7 ns por tique |
| Troca de contexto | `bench` e a linha `switch:`: ping-pong de `unpark`/`park` entre duas tarefas, ns por troca, aquecimento e melhor de 3 | aarch64 HVF `release`, 5 boots: **41 a 44 ns** na linha do boot (antes 68 a 75) e **42 ns** no comando (antes 73 a 158); a tabela e a decomposição estão em "Otimização da troca e do acordar", abaixo; x86_64 e aarch64 em TCG só para comparar o RustOS consigo mesmo |
| Jitter de acordar | `bench`: `agora − pedido` depois de `sleep_until`, 200 amostras | aarch64 HVF, com o EEVDF: 0,49 a 0,52 ms (ociosa em `wfi`) e 4,1 µs (CPU ocupada) de média; igual ao do rodízio (tabela abaixo) |
| Latência de acordar entre tarefas | `bench` (linha `task to task`) e a linha `wake self-test:` do boot: `unpark` de uma tarefa de fatia curta por outra, na mesma CPU, até ela rodar  | aarch64 HVF: mediana de **dezenas de ns, no piso do contador** (24 MHz = 41,7 ns por tique: 42 a 83 ns são 1 a 2 tiques), antes 102 µs |
| Timer (aarch64) | linha `timer:` do boot; `CNTV` com `CVAL` absoluto, 100 Hz só até `task::init`; depois não há tique periódico (linha `idle:`) | 50 tiques em 497 ms (HVF) e 488 ms (TCG) no autoteste; ociosa com o EEVDF: 2 IRQs do timer em 50 ms, contra 5 a 100 Hz (HVF, TCG e x86_64 iguais). No x86_64 com o contador do APIC (sem TSC-deadline) a ociosa ainda acorda a cada ~69 s, o que a contagem de 32 bits a 62,5 MHz alcança; no aarch64 e no TSC-deadline o prazo é de uma hora |
| Estado do gerador aleatório | `size_of::<Rng>()` (teste em `core/crypto/src/rng.rs`) | 40 bytes: chave de 32 e contador de 8 |
| Resíduo de chave na pilha | linha `key residue:` do boot (`random::stack_residue`), `make smoke ARCH=all PROFILE=dev\|release\|release-small` | 0 estados do ChaCha20 e 0 cópias da chave nas duas ISAs e nos três perfis; antes, de 0 a 2 e de 1 a 3 (tabela abaixo, no gerador) |
| Código de `core/crypto` | `.text` do crate sem LTO, com todas as funções públicas (`llvm-size`); no kernel, com LTO, só entra o que se usa | `release`: 8 202 (x86_64) e 7 866 (aarch64) bytes · `release-small`: 6 216 e 7 349 bytes |
| Vazão de ChaCha20, BLAKE2b e do gerador | `cargo run --release -p rustos-crypto --example bench`, no host (Apple M5, `rustc` 1.100.0), melhor de 15 rodadas, sem SIMD (o compilador não vetorizou: o resultado com `-vectorize-loops=false -vectorize-slp=false` é o mesmo) | Ver tabela abaixo. **Números do host**; os do alvo estão abaixo |
| Tabela de handles e canal (`core/object`) | `cargo test --release -p rustos-object bench -- --ignored --nocapture --test-threads=1` (`core/object/src/tests/bench.rs`), no host | Host: buscar com direito 0,6 ns; inserir, remover e fechar 8,2 ns; escrever e ler uma mensagem de 64 B com 4 handles, cerca de 125 ns; slot de 32 bytes, par de pontas de 7 256 bytes ([seção](#tabela-de-handles-e-canal-fatia-4c-medido-no-host)). **Medida de host**, não do alvo |
| Escalonador (`core/sched`) | `cargo test --release -p rustos-sched bench -- --ignored --nocapture --test-threads=1` (`core/sched/src/bench.rs`), no host | Host: `pick_next` do EEVDF em 41 a 136 ns com 8 a 512 tarefas; 96 bytes por tarefa ([SDD 14](14_escalonador.md)). No alvo (`bench`, 8 prontas): `pick_next` 34 a 35 ns e `tick` 2 ns no aarch64 HVF; 320 e 36 ns em TCG; 670 a 730 e 85 a 89 ns no x86_64 em TCG (tabela abaixo) |
| Boot de parede | `make smoke-fast` (modo normal) e `make smoke` (modo de teste) | inclui firmware EDK2: 1,7 a 1,9 s (x86_64, TCG) e 0,17 s (aarch64, HVF; 0,6 s em TCG) no modo normal, dominado pelo firmware (no aarch64, antes de `-boot menu=on,splash-time=0`, eram 5,2 s, [SDD 05](05_build_e_execucao.md#qemu-e-firmware)); a tabela de [Tempo de boot](#tempo-de-boot) separa firmware, Limine e kernel |
| Tempo do kernel até o prompt | linha `boot time:` do boot (`make smoke-fast ARCH=all PROFILE=release`) | modo normal: 25 a 31 ms (x86_64, TCG) e 2 ms (aarch64, HVF); antes, com os autotestes e medidas no caminho: 964 e 453 ms ([Tempo de boot](#tempo-de-boot)) |

### Syscall nula e limpeza da pilha (fatia 4a)

Como se mede: o caso `null-loop` do programa `user/tests/cases` (`make smoke ARCH=…`, linha `user: null
syscall`): o programa faz 200 000 chamadas `nop` (20 000 no x86_64, em TCG) e lê o contador
no usuário (`cntvct_el0`, `rdtsc`); o kernel converte e divide. Mediana de 5 rodadas **alternadas**
entre as configurações, a deriva da máquina cai igual em todas; o processo roda numa CPU só, sem
migração. aarch64 sob **HVF** (`RUSTOS_ACCEL=hvf`), QEMU 11.1.3, Apple Silicon, 256 MiB. O x86_64 só
roda em **TCG**: o número serve de regressão do RustOS contra ele mesmo, nunca de latência
([guidelines 05](../guidelines/05_recursos_e_desempenho.md)).

| | Com a limpeza da pilha | Sem a limpeza | Saída por `IRETQ` em vez de `SYSRET` |
| --- | --- | --- | --- |
| aarch64 HVF, `release`, 3 rodadas do comando | 49 a 51 ns | 37 a 41 ns | não existe |
| aarch64 HVF, `dev`, 2 rodadas | 52 ns | 41 a 42 ns | não existe |
| x86_64 TCG, `release` | 473 a 494 ns | 273 a 279 ns | 529 a 564 ns |
| x86_64 TCG, `dev` | 492 ns | 252 ns | 542 ns |

- **A limpeza custa de 10 a 12 ns por syscall no aarch64** (a chamada nula sem ela leva 37 a 41 ns:
  um `SVC` e um `eret` com o quadro de 288 bytes) e **cerca de 200 ns no TCG do x86_64**, onde o
  `rep stosq` e as leituras são emulados. Pesa em relativo e é pouco em absoluto. A versão
  anterior da heurística varria 128 palavras limpas e repintava 512 bytes: custava 30 a 70 ns, e
  baixou ao reduzir a faixa limpa para 16 palavras e a janela do `arch/` para 256 bytes (as
  medidas intermediárias: 72 ns, 33 ns e 17 ns de custo). O que a limpeza deixa de ver está na
  [DEBT-0073](../tech-debt/0073-limpeza-da-pilha-so-ate-128-bytes-de-limpo.md).
- **`SYSRET` ganha de 50 a 70 ns sobre o `IRETQ` no TCG**; fica ligado, com o `IRETQ` de reserva
  quando a conferência de `RCX`, `R11`, seletores, `RIP < USER_TOP` e `RF`/`TF` não passa. A
  vantagem em metal é maior (o `IRETQ` é lento), e não foi medida (DEBT-0014, DEBT-0041).
- **Efeito no boot:** o boot normal não roda processo de usuário. `make smoke-fast ARCH=all
  PROFILE=release`, 5 boots, linha `boot time:`: x86_64 25 a 26 ms (antes da 4a, 30 e 32; a diferença
  é da máquina, não da mudança) e aarch64 2 a 3 ms (antes, 2 a 3). `release-small`: 26 a 28 ms e 2 ms
  (antes, 32 e 3).
- **Pico de pilha com a 4a** (`user: ... kernel stack depth`, `release`): syscall de 1 912 B
  (x86_64) e 2 080 B (aarch64) de 16 384 (a `debug_write`, que formata no console); falta fatal
  de usuário de 1 544 B e 1 656 B. **Os 16 KiB ficam** (12,7% da pilha, longe dos 75% que o gate
  de `stacks:` reprova); a [DEBT-0070](../tech-debt/0070-pilha-de-16-kib-sem-syscall-nem-falta-de-usuario.md)
  segue aberta até os handles, os canais e a falta resolvida.
- **Memória:** a IST4 do `#DB` custa 16 KiB de pilha mais a guarda por CPU no x86_64; o processo
  mínimo (`hello`) gasta 24 frames (96 KiB: a raiz, 3 tabelas para o código e 3 para a pilha, 1 página de
  código e 16 de pilha) mais 16 KiB de pilha do kernel, e devolve os frames zerados ao morrer.

### Clamp do Spectre v1 na syscall (fatia 4e, parte 1)

Como se mede: o mesmo da [seção anterior](#syscall-nula-e-limpeza-da-pilha-fatia-4a) (`make smoke`,
linha `user: null syscall`, `release`), cinco rodadas **alternadas** entre as variantes, cada
número já a mediana de 5 dentro do autoteste. aarch64 sob **HVF** (`RUSTOS_ACCEL=hvf make smoke
ARCH=aarch64 PROFILE=release`, QEMU 11.1.3, Apple M5); x86_64 em TCG, que só compara o RustOS
consigo mesmo. As variantes: **A** a `main` (`Syscall::from_raw`, que percorre `Syscall::ALL`), **B** a tabela
indexada pelo número, sem clamp, **D** a primeira versão do clamp (máscara aritmética do `hal` mais
`CSDB`), **E** a que entra (a sequência do Linux em `asm!` em `arch/`: `cmp; sbb` no x86_64,
`cmp; sbc; csdb` no aarch64).

| Syscall nula, com a pilha limpa | A (`main`) | B (tabela) | D (máscara do `hal` + `CSDB`) | E (sequência do Linux) |
| --- | --- | --- | --- | --- |
| aarch64 HVF, `release`, 5 rodadas | 46,3 a 47,0 ns | 47,3 a 47,6 ns | 57,8 a 60,0 ns | **57,3 a 57,7 ns** |
| x86_64 TCG, `release`, 5 rodadas | 414,0 a 421,9 ns | 421,9 a 429,3 ns | não medida | **423,9 a 432,1 ns** |

- **No aarch64 a `CSDB` custa cerca de 10 ns** (E contra B: 57,5 contra 47,5 ns; a máscara `cmp; sbc`
  sozinha não aparece, como na primeira medida, em que a variante sem `CSDB` ficou no ruído). É +21% na
  chamada nula, que sem a limpeza da pilha vai de 37 para 48 ns. D e E não se distinguem: trocar a
  aritmética do `hal` pela sequência do Linux não mudou o número. O valor é do Apple M5; outros núcleos não foram medidos
  ([DEBT-0105](../tech-debt/0105-csdb-do-clamp-custa-10-ns-sem-o-requisito-lido.md)). **A `CSDB` fica**:
  o Linux a põe incondicional (`arch/arm64/include/asm/barrier.h`) e tirá-la sem ler o requisito é
  desligar segurança sem prova ([guidelines 09](../guidelines/09_seguranca.md)); este é o número que
  a prova pede.
- **A tabela indexada custa o mesmo que o laço** para `nop`, a primeira entrada de `Syscall::ALL`
  (B contra A). Para as chamadas do fim da lista é mais barata, e não foi medida.
- **x86_64 TCG:** E fica cerca de 7 ns (1,6% na mediana das cinco rodadas: 425,1 contra 418,3 ns) acima de A, e a tabela sozinha (B) já está uns 10 ns acima; a diferença é a do código que o TCG emula, e a faixa do B
  (acima do E) mostra o quanto ele varia com o layout.
- **Boot normal:** `make smoke-fast ARCH=all PROFILE=release`, 5 boots, linha `boot time:`: x86_64
  25 a 26 ms antes e 27 depois (cinco vezes 27); aarch64 2 a 3 ms antes e 2 a 3 depois. A linha
  `mitigations:`, o CPUID de cada um dos 4 núcleos e a leitura do MSR custam cerca de 1 ms no x86_64 em
  TCG (o CPUID é emulado) e não aparecem no aarch64.
- **Tamanho** (`make size`, `carregado`, sobre a `main` `c42f274`): `release` x86_64 371 635 para 377 307
  bytes (+5 672) e aarch64 314 265 para 318 053 (+3 788); `release-small` x86_64 195 367 para 197 895
  (+2 528) e aarch64 177 329 para 179 749 (+2 420). É a detecção, as listas, a formatação da linha, a
  união por núcleo e a tabela de despacho (536 bytes de `.rodata`). O `.bss` cresceu 16 bytes (o
  lock da união).

### PCID e ASID na troca (fatia 4e, parte 2)

Como se mede: `make smoke` (modo de teste, perfil `dev`), linha `asid bench:`: ida e volta entre dois espaços,
cada visita lê uma página; ns por troca, média de 2 000 trocas depois de aquecer. Na **`main`** a mesma medida
roda como cópia temporária, sem o módulo de etiquetas (não commitada, a árvore foi restaurada). Linha de
`null syscall` do mesmo smoke, com a pilha limpa, mediana das três ou cinco rodadas. Aceleração de cada número:
**HVF** (`make smoke ARCH=aarch64`, Apple Silicon, QEMU), **TCG** (`RUSTOS_ACCEL=tcg`, emulado; só compara o
RustOS consigo mesmo) e **x86_64 em TCG** (sem KVM).

| Medida | `main` (sem etiqueta) | Branch | Aceleração |
| --- | --- | --- | --- |
| Troca entre dois espaços, aarch64, 5 boots | 543 a 991 ns (mediana 767) | **57 a 84 ns** (mediana 60) | HVF |
| Troca entre dois espaços, aarch64, 3 boots | 7 483 a 7 679 ns (mediana 7 522) | 9 876 a 11 455 ns (mediana 10 887) | TCG |
| Troca entre dois espaços, x86_64, 3 boots | 9 277 a 9 947 ns (mediana 9 945) | 8 966 a 9 204 ns (mediana 9 171) | TCG, **sem PCID** nos dois |
| `null syscall`, aarch64, com a pilha limpa, 5 boots | 67,7 a 78,6 ns (mediana 69,8) | 67,5 a 68,1 ns (mediana 67,7) | HVF |
| `null syscall`, aarch64, com a pilha limpa, 3 boots | 1 383 a 1 507 ns (mediana 1 414) | 1 466 a 1 578 ns (mediana 1 500) | TCG |
| `null syscall`, x86_64, com a pilha limpa, 3 boots | 712 a 830 ns (mediana 816) | 774 a 939 ns (mediana 855) | TCG, sem PCID nos dois |

- **aarch64 em HVF: a troca cai cerca de 13 vezes** (767 para 60 ns). É o efeito que a fatia buscava: a troca
  deixa de fazer `tlbi vmalle1`, e a TLB das duas raízes sobrevive. O número é do Apple Silicon deste host; outros
  núcleos não foram medidos.
- **aarch64 em TCG: a troca do branch é mais lenta** (mediana 10 887 contra 7 522 ns, cerca de 45%). **NÃO EXPLICADO**:
  o TCG não tem o custo real de `tlbi`, então o ganho de HVF não aparece aqui, e o trabalho extra que o branch faz
  (o `for_switch` com o lock do gerador, a escrita do `Shared` e a conferência do `tlb_gen`) é o que sobra nesse
  modelo. Não se trata como regressão aceita sem olhar: é o próximo número a investigar, e a DEBT-0076 (o lock do
  gerador) é o primeiro suspeito a medir.
- **x86_64 em TCG não mede PCID**: o TCG não tem PCID nem INVPCID, e `PCID_ENABLED` está desligado
  ([DEBT-0155](../tech-debt/0155-pcid-x86-sem-execucao.md)). Os dois lados rodam o caminho sem etiqueta. As faixas
  não se cruzam (o branch fica 8% abaixo da mediana da `main`), e nenhuma mudança da fatia explica isso no caminho
  sem etiqueta: **NÃO EXPLICADO**, provavelmente variância do host, a conferir na próxima medida.
- **`null syscall`**: no aarch64 em HVF a chamada nula não muda (67,7 contra 69,8 ns, dentro do ruído); no TCG a
  diferença de 6% fica na faixa de ruído das duas medidas. No x86_64 a chamada não passa pelo caminho novo.
- **Tamanho** (`make size ARCH=x86_64 PROFILE=release-small`, antes na `main`, depois no branch): `carregado`
  277 527 para 278 175 bytes (+648, +0,2%); `.bss` 19 009 para 19 017 (+8 bytes); `arquivo` 374 624 para 374 960.

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
| Acordar entre tarefas, mediana (`wake self-test:`) | 102 µs (a fatia de 100 µs do dorminhoco, não uma latência geral) | **dezenas de ns, no piso do contador** (42 a 83 ns) |
| Acordar de `sleep_until`, ociosa (média, µs) | 472 a 514 | 504 a 562 (igual: é o `wfi` do HVF) |
| `pick_next` e `tick` da política, 8 prontas (ns) | 34 a 39 e 2 | 35 a 39 e 2 |
| x86_64 TCG, troca, linha do boot (ns), 3 boots | 1 398 / 1 519 | 1 294 / 1 307 |
| x86_64 TCG, troca, comando `bench` (ns), 3 boots | 1 595 / 1 603 | 1 276 / 1 282 |
| x86_64 TCG, acordar entre tarefas, mediana (`wake self-test:`) | não medido | 1 a 3 µs (a resolução do relógio do TCG); com `RUSTOS_SMP=1` a troca de contexto inteira sai a 30 µs (`dev`) ou ~10 µs (`release`) em parte dos boots, nas duas árvores: `dev` 8 de 64 na `main` e 9 de 64 aqui; `release` 9 de 12 e 10 de 12 (medida do revisor; SDD 15) |

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

**Acordar entre tarefas:** antes, `unpark` só levantava o `needs_resched` e a corrente seguia até a próxima IRQ (102 µs
medidos, mediana de três boots: 102 375, 102 458 e 102 417 ns; máximo de 107 a 172 µs; é a fatia de 100 µs do dorminhoco, e não uma
latência de acordar em geral). Agora troca no fim do `unpark`: 42 a 83 ns
de mediana, média de 54 a 72 ns e máximo de 125 a 250 ns nos boots medidos; uma amostra em umas 350 (4 em ~1 700) passa
de 350 µs (lag negativo do dorminhoco: o EEVDF não o deixa preemptar). A resolução do contador (24 MHz = 41,7 ns) é do tamanho do que se mede: o número é "dezenas de ns, no piso do contador", não um valor preciso.
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

### Árvore de regiões e gerador de ASID (fase 4, medido no host)

Comando: `cargo test --release -p rustos-mm bench_vmar -- --ignored --nocapture
--test-threads=1` (`core/mm/src/bench_vmar.rs`; o filtro `bench` do comando da seção anterior
também o roda). Máquina: Apple M5 (aarch64), rustc 1.100.0, `release` (LTO), melhor de 7 rodadas, mesma
máquina do resto desta seção, com carga alheia (load average 5,5). **Medida de host, não de
QEMU nem de metal.** N regiões de 1 página nas páginas pares, buracos de 1 página entre
elas; `Vmar<Vec<Region>>`. ns por operação, cinco execuções do comando (a faixa é o mínimo
e o máximo delas):

| N | `find` | `check_fault` | `reserve` + `unmap` (par, no meio) | `allocate` + `unmap`, `Lowest` | idem, `Random` |
| --- | --- | --- | --- | --- | --- |
| 16 | 10 | 9 a 10 | 36 | 36 a 37 | 66 a 68 |
| 256 | 16 | 16 | 120 a 124 | 442 a 450 | 905 a 1 002 |
| 4 096 | 24 a 28 | 24 a 25 | 1 088 a 1 105 | 6 830 a 6 970 | 14 060 a 14 250 |

- **Achar é logarítmico** (10 para 24 ns de 16 a 4 096 regiões) e é o caminho da falta de
  página: a decisão inteira (`check_fault`) custa o mesmo que `find`.
- **Reservar e soltar é linear** pelo deslocamento: o par custa 36 ns com 16 regiões e
  1,1 µs com 4 096 (em média 64 KiB movidos duas vezes). O `init` terá dezenas de regiões
  (inferência): ali o par fica na casa de dezenas de ns.
- **`allocate` mede o pior caso**: nenhum buraco serve, então varre todos os N + 1 (a
  região livre fica depois da última). Com buraco cedo, `Lowest` para no primeiro. `Random`
  varre duas vezes (conta, depois localiza) e custa 2x.
- **Memória:** 32 bytes por região (`size_of::<Region>() == 32`, verificado em tempo de
  compilação): 512 B com N = 16, 8 KiB com 256, 128 KiB com 4 096. O `AsidAllocator` custa 672
  bytes com PCID de 12 bits e 8 CPUs.
- Uma compilação anterior do mesmo algoritmo mediu `allocate` em 2 217 ns (`Lowest`) e
  4 888 ns (`Random`) com N = 4 096: a varredura é sensível ao que o compilador faz com
  `lo()` e `hi()`. Não foi investigado; os números da tabela são os da versão que entrou.

O `AsidAllocator` não tem medida de ciclos: o custo que importa é o da 4e (o `INVPCID`, o
`tlbi` e o lock), que só vale sob KVM/HVF ou metal.

### Tabela de handles e canal (fatia 4c, medido no host)

Comando: `cargo test --release -p rustos-object bench -- --ignored --nocapture --test-threads=1`
(`core/object/src/tests/bench.rs`). Máquina: Apple M5 (aarch64), `rustc` 1.100.0, perfil `release`,
melhor de 9 rodadas de 200 000 chamadas, três execuções. **Medida de host**: não é QEMU nem metal, e
o kernel real troca o `Mutex` do teste pelo `IrqLock`; serve para comparar o RustOS consigo mesmo
(guidelines 05). Os números do alvo entram quando a 4c ligar a tabela.

| O quê | Host | Observação |
| --- | --- | --- |
| Buscar com direito (`get`, 1 001 handles, 7 de passo) | 0,6 ns (uma execução com 1,8 ns) | Vazão de buscas independentes: o laço não encadeia as chamadas |
| Inserir, remover e fechar | 8,2 a 8,6 ns | Inclui o `clone` do `Arc` do teste, 3,2 ns sozinho |
| Escrever e ler 1 mensagem de 64 B com 4 handles, mais preparar e fechar os 4 | 183 a 193 ns | Cinco locks do `Mutex` sem disputa, os `Arc` dos bytes e dos handles |
| Só preparar e fechar os 4 handles (sem canal) | 59 a 61 ns | Por diferença, escrever e ler custam cerca de 125 ns |

**Memória por unidade** (`size_of`, o teste confere o do slot): `Slot` com o `T` do kernel (enum de seis
`Arc`s), **32 bytes**; pior caso de uma tabela cheia, 4 096 × 32 = **128 KiB** por processo (o `Vec`
do kernel pode chegar ao dobro pela capacidade); `Message` **224 bytes**; `Channel` (par de pontas,
16 mensagens por ponta inline) **7 256 bytes**, ocupados ou não ([DEBT-0091](../tech-debt/0091-fila-do-canal-com-armazenamento-inline.md)).

### Handles, canais e espera no kernel (fatia 4c, parte 1)

Comando: `make smoke ARCH=<isa> PROFILE=release` (aarch64 com `RUSTOS_ACCEL=hvf`, QEMU 11.1.3, Apple M5; x86_64 em
TCG, que só compara o RustOS consigo mesmo), cinco rodadas **alternadas** entre a `main` (`2083547`) e o branch.
O programa `handles` (`user/tests/handles`) mede no mesmo processo, com `clock_get_monotonic`, 2 000 chamadas nulas,
1 000 pares `handle_duplicate` + `handle_close` e 1 000 idas e voltas de 64 B (`channel_write`, `channel_read`,
`channel_write`, `channel_read`: 4 chamadas), linha `[user] handles: null call …`. Ainda não há dois processos, então a
ida e volta é na mesma tarefa, sem troca de contexto; a troca entre processos é da parte 3.

| O quê | aarch64 HVF, `release` | x86_64 TCG, `release` |
| --- | --- | --- |
| Syscall nula, pilha limpa, `main` (linha do kernel) | 58,1 a 61,3 ns | 479,2 a 483,8 ns |
| Syscall nula, pilha limpa, branch (linha do kernel) | 58,8 a 63,2 ns | 464,5 a 472,2 ns |
| Chamada nula medida pelo `handles` (a mesma conta, em usuário) | 59,8 a 64,0 ns | 466,9 a 490,6 ns |
| `handle_duplicate` + `handle_close` | 169,6 a 184,5 ns | 1 444 a 1 476 ns |
| Ida e volta de 64 B por um canal (4 chamadas) | 1 393 a 1 493 ns | 15,0 a 18,3 us |

- **A syscall nula não mudou** (mesma faixa nos dois, dentro do ruído do TCG): o despacho ganhou entradas na tabela, e
  as chamadas novas não mexem no caminho de `nop`.
- Uma chamada de canal custa cerca de 350 ns no aarch64 (o `Arc<[u8]>` dos bytes, dois `IrqLock`, a comparação de
  sinais antes e depois, a cópia de usuário). Sem número de antes: as chamadas são novas; o de comparação é o da
  chamada nula (cerca de 60 ns) e o do host ([tabela de handles e canal](#tabela-de-handles-e-canal-fatia-4c-medido-no-host)).
- **Menor processo**, linha `user: init load` (`release`): 32 frames antes e **35** depois (21 de dados, 7 de tabelas, 7 do kernel: pilha,
  FPU e heap), e o `init` carrega em 3 `PT_LOAD` nas duas ISAs (eram 3 no x86_64 e 2 no aarch64). Os 2 frames do kernel a mais são o
  heap que cresceu pelo canal de boot (um par de 7 256 bytes, mais objetos e tabela) e 1 de dados é o `init` maior. Tempo até a
  primeira chamada: aarch64 25 para 33 us; x86_64 TCG 3,2 para 5,8 ms (código frio em TCG, varia até 2x).
- **Pilha do kernel** (`user: ... kernel stack depth`, `stacks:`, `release`): fundo numa syscall de 1,8 para 4,2 KiB (x86_64) e de 2,0 para
  4,4 KiB (aarch64); pico das tarefas de 2,2 para 4,1 KiB (x86_64) e de 2,4 para 4,3 KiB (aarch64), de 16: abaixo de 8 KiB, e as 16 KiB
  ficam ([DEBT-0070](../tech-debt/0070-pilha-de-16-kib-sem-syscall-nem-falta-de-usuario.md)). A maior parte é o `channel_write` e o `channel_read`, que
  copiam até 1 KiB de bytes e 512 de handles pela pilha.
- **Tamanho do kernel** (`make size`, antes e depois): `release` x86_64 `carregado` de 388,5 para **437,1 KiB** (`text` 316,8 para 355,3;
  `rodata` 38,6 para 47,9, dos quais 7 KiB do modelo do par de canal), aarch64 de 326,1 para **374,2 KiB**; `release-small` x86_64 de 193,4
  para **217,9 KiB** e aarch64 de 175,7 para **199,3 KiB**. É o custo do `core/object` instanciado para o `Obj`, das chamadas e dos `Arc`.
  A imagem enxuta paga o código mesmo sem rodar processo (o despacho o liga): reduzir é medida futura (instanciação
  genérica menor, `channel_read` e `channel_write` sem o buffer de 1 KiB na pilha), sem número ainda.
- **Memória por unidade:** um canal custa 7 256 bytes de heap de fila mais os dois `Arc` de ponta (cerca de 100 bytes); um processo, a tabela de handles
  (`Vec` de slots de 32 bytes, que cresce de um em um: o `init` usa 3 handles) e o `ProcessObj`. A cobrança limita um processo a 256 pontas, e o teto vale em
  **pontas, não em bytes**: 128 pares de 7 256 bytes são **0,89 MiB de filas**, mais até cerca de **4 MiB de mensagens** (128 pares × 2 pontas × 16
  mensagens × 1 KiB). Um processo hostil medido por revisor chegou a **17,7 MiB** de pico, e 127 ciclos entre canais (cerca de 1,5 mil
  syscalls) **vazam 18,3 MiB para sempre** ([DEBT-0090](../tech-debt/0090-ciclos-entre-filas-de-canais-nao-coletados.md),
  dívida 0110 (paga na parte 3)): a parte 3, com `process_create`, precisa de um teto em **bytes** por processo.
- **O custo do `EMPTY_PAIR`** (o modelo de 7 KiB que existe porque a fila é inline, [DEBT-0091](../tech-debt/0091-fila-do-canal-com-armazenamento-inline.md)):
  sem ele a imagem seria **6,8 KiB menor no x86_64 e 6,6 KiB no aarch64** (medido pelo revisor, `release`). Sai quando a fila deixar de ser inline.

### Processo mínimo e carregador do init (fatia 4b)

Medido na QEMU 11.1.3 num Mac com Apple Silicon: aarch64 em HVF, x86_64 em TCG (só se compara o RustOS consigo mesmo). `make smoke`
no modo de teste (a linha `user: init load`), `release-small` com `RUSTOS_SELFTEST=1`. O `init` roda só no modo de teste
([DEBT-0087](../tech-debt/0087-init-so-no-boot-de-teste.md)); o carregador entrou na imagem de produção com o M1 ([abaixo](#o-carregador-na-imagem-enxuta-m1)).

| Medida | x86_64 (TCG) | aarch64 (HVF) |
| --- | --- | --- |
| Tamanho do `init` (`release-small`, arquivo ELF) | 17 600 bytes | 14 632 bytes |
| Segmentos do `init` (`PT_LOAD`) | 3: R (cabeçalhos e `.rodata`), RX (5 135 bytes de código), RW | 2 |
| Frames do menor processo | **32** (128 KiB): 20 de dados (4 de imagem, 16 de pilha), 7 de tabelas (raiz, mais três níveis para a imagem e três para a pilha) e 5 do kernel (pilha da tarefa de 4 páginas, mais o que o heap guarda: a área de FPU, o `Process` e as 32 regiões, 1 KiB) | **31**: 19 de dados, 7 de tabelas, 5 do kernel |
| Tempo do `cpio` até a entrada em usuário | 3 a 6 ms (o código do carregador roda frio no TCG; mais o acordar de outra CPU) | 22 a 41 µs |
| Tempo do `cpio` até a primeira syscall | 3,3 a 6,3 ms | 23 a 42 µs |
| Fase `init` do boot de teste (carregar, rodar, imprimir, sair e esperar) | 10 a 13 ms | 1,3 a 1,8 ms (a maior parte é a linha na serial e na tela) |
| `big` (1,3 MiB de arquivo, 2,3 MiB mapeados) | sai com 0 | sai com 0 |

Os 16 frames da pilha eram metade do menor processo: saíram com a falta de página resolvida na 4c ([abaixo](#paginação-sob-demanda-e-shootdown-fatia-4c)).

**`boot time:` do boot normal, antes e depois: sem mudança**, porque o boot normal não lança o `init`. `release-small`, 3 boots em cada lado: aarch64
**2 ms antes e 2 ms depois** (`2 ms`, `2 ms`, `2 ms` em cada); x86_64 em TCG **34, 39 e 42 ms antes; 31, 40 e 60 ms depois** (o TCG oscila de um boot para o
outro, [DEBT-0068](../tech-debt/0068-boot-de-teste-do-x86-em-tcg-bimodal-por-layout.md), e o caminho do boot normal não mudou).
**O experimento** com o `init` no boot normal (descartado): aarch64 de 2 para 4 a 7 ms, x86_64 em TCG de 34-42 para 38-42 ms com a fase `init` em 10 a 13 ms.

**`make size`, `release-small`, antes e depois.** O carregador e o código de usuário ficam fora da imagem enquanto o `init` só roda no modo de teste:

| | x86_64 antes | x86_64 depois | aarch64 antes | aarch64 depois |
| --- | --- | --- | --- | --- |
| `text` | 127 991 | 128 039 | 115 820 | 115 912 |
| `rodata` | 33 101 | 33 149 | 28 669 | 28 781 |
| `data` | 18 434 | 18 482 | 18 216 | 18 288 |
| `bss` | 15 841 | 15 841 | 14 624 | 14 624 |
| carregado | 195 367 | 195 511 | 177 329 | 177 605 |

(+144 bytes no x86_64 e +276 no aarch64, 0,07% e 0,16% do carregado. **Causa**, de `llvm-nm -S` contra um build da `3acc35e`: o carregador, o `Vmar` e o código de usuário não entram na imagem; o que muda é o caminho de morte do processo, que a syscall `process_exit` da 4a mantém vivo: `process::terminate` agora solta a árvore de regiões do `Process`, `UserSpace` ganhou dois contadores de frames (`data_frames` e `table_frames`) e `map_at` e `destroy` os usam. O resto é reordenação de genéricos, com símbolos de até 300 bytes a mais e a menos, e preenchimento de alinhamento.)

### Paginação sob demanda e *shootdown* (fatia 4c)

Medido na QEMU 11.1.3 num Mac com Apple Silicon: aarch64 em HVF, x86_64 em TCG (só se compara o RustOS consigo mesmo). `make smoke ARCH=all PROFILE=release`, modo de teste; o `user/tests/lazy` mede por página, com `clock_get_monotonic`, três laços de 4 096 páginas (16 MiB) cada, e o autoteste mede o *shootdown*. Três execuções por ISA, mais as do revisor independente (a faixa é o mínimo e o máximo). **O tempo por página inclui a entrada e a saída da exceção e o tratador inteiro.** A [DEBT-0058](../tech-debt/0058-zerar-pagina-no-x86-sem-medida-em-hardware.md) segue aberta: o do x86_64 é de TCG e não decide `stosq` contra `stosb`.

| Por página | x86_64 (TCG) | aarch64 (HVF) |
| --- | --- | --- |
| Primeiro toque de escrita (aloca e zera um frame, mapeia) | 11,5 a 11,6 µs | 0,51 a 0,69 µs |
| Leitura de página não tocada (mapeia a página zero) | 1,33 a 1,37 µs | 0,150 a 0,152 µs |
| Escrita sobre a página zero (aloca, zera, troca a folha, invalida) | 3,28 a 3,43 µs | 0,70 a 1,11 µs |
| Invalidar uma página nas outras CPUs (x86_64: IPI de ida e volta a uma CPU, 200 invalidações; aarch64: `tlbi vaale1is` e `dsb ish`, 2 000) | 8,0 a 8,3 µs (uma medida do revisor, 30,7 µs) | 0,13 a 0,32 µs |

- A leitura custa o décimo da escrita porque não aloca nem zera. O aarch64 gasta 0,15 µs só para entrar na exceção, achar a região, mapear a página zero e voltar.
- No x86_64 o primeiro toque custa o triplo da cópia da página zero, embora as duas aloquem e zerem um frame. **Hipótese, não medida:** o primeiro laço toca memória da máquina que o host da QEMU ainda não paginou (a primeira escrita num frame novo do convidado é uma falta do host); a cópia reaproveita frames que o primeiro laço já tocou. Não afeta o aarch64 em HVF.
- O IPI de ida e volta de 8 µs bate com o `ipi round trip` de 8 a 10 µs da seção de SMP deste capítulo (o mesmo TCG): o custo é a QEMU entregar a interrupção à outra CPU, não o `invlpg`. Só vale para comparar o RustOS consigo mesmo ([DEBT-0122](../tech-debt/0122-shootdown-sem-medida-em-kvm-nem-lote.md)).
- O controle do teste (a parte remota desligada) deixa a tradução velha viva no x86_64 em TCG (26 553 leituras velhas em 120 000, no anel) e no aarch64 em TCG (`RUSTOS_ACCEL=tcg RUSTOS_CPU=a64fx`, o último passo do `make pr`); no aarch64 em HVF **não** tem dentes (0 em 240 000). O gate do aarch64 só distingue `is` de local em TCG.
- **aarch64 em TCG** (uma execução, a do `make pr`): primeiro toque 13,4 µs, leitura 3,1 µs, cópia da página zero 25,9 µs e 4,0 µs por invalidação. A cópia custa 8 vezes o primeiro toque porque o `tlbi ...is` esvazia o TLB de software de todas as CPUs da QEMU; no HVF o mesmo `tlbi` custa 0,13 a 0,21 µs.

**Memória do menor processo (o `init`), antes e depois**, linha `user: init load` (`make smoke`, as duas ISAs dão o mesmo):

| | Antes (a `main` `2083547`) | Depois |
| --- | --- | --- |
| Frames na criação | **32** (128 KiB) | **17** (68 KiB) |
| De dados | 20 (4 de imagem, 16 de pilha) | 5 (4 de imagem, 1 de pilha: a dos argumentos) |
| De tabelas | 7 | 7 |
| Do kernel (pilha da tarefa, FPU, heap) | 5 | 5 |
| Ao sair | — | 5 de dados e 7 de tabelas: o `init` não toca as outras 15 páginas da pilha |

Os 15 frames a menos são 47% do menor processo. A página zero custa 1 frame para a máquina inteira, criado na primeira leitura de página nunca tocada.

**Custo por unidade** (guidelines 05): a região preguiçosa não custa byte (`Region` segue com 32 bytes: o `lazy` cabe no preenchimento de `Attrs`); o espaço ganhou 20 bytes (a máscara de CPUs e os dois contadores de páginas preguiçosas; `UserSpace` tem 56 bytes); as caixas do *shootdown* custam 8 x 8 x 24 = 1 536 bytes de `.bss` no x86_64 e nada no aarch64.

**`boot time:` e `make size`, `release-small`, antes e depois.** O boot normal não lança processo nem toca a falta de página; os 3 boots de cada lado:

| | x86_64 (TCG) | aarch64 (HVF) |
| --- | --- | --- |
| `boot time:` antes (`2083547`) | 28 e 28 ms | 2 e 3 ms |
| `boot time:` depois | 32 e 30 ms (o TCG oscila de um boot para o outro, [DEBT-0068](../tech-debt/0068-boot-de-teste-do-x86-em-tcg-bimodal-por-layout.md)) | 2 e 3 ms |

| `make size` | x86_64 antes | x86_64 depois | aarch64 antes | aarch64 depois |
| --- | --- | --- | --- | --- |
| `text` | 129 815 | 132 551 | 117 220 | 119 344 |
| `rodata` | 33 901 | 34 253 | 29 813 | 30 021 |
| `data` | 18 482 | 18 922 | 18 288 | 18 632 |
| `bss` | 15 857 | 17 409 | 14 640 | 14 656 |
| carregado | 198 055 | 203 135 | 179 961 | 182 653 |

(+5 080 bytes no x86_64 e +2 692 no aarch64, 2,6% e 1,5% do carregado. No x86_64, 1 536 do `bss` são as caixas do *shootdown*; o resto é o resolvedor da falta, a reserva de páginas, o `tlb` e o `destroy` que espera a CPU, que a imagem enxuta carrega porque a morte do processo é a mesma.)

### O carregador na imagem enxuta (M1)

Medido em 2026-11-19, `make size` e `make smoke-fast PROFILE=release-small`, rustc 1.100.0, QEMU 11.1.3 (x86_64 em TCG, aarch64 em HVF, `-smp 4`, 256 MiB). "Antes" é a `main` `03f5849`; "depois" é o PR do M1. Até aqui o compilador descartava da `release-small` o carregador, a árvore de regiões, a paginação de usuário, os handles e a espera (nada os chamava fora do modo de teste); o M1 os chama pelo `run` do shell, então entram.

| `make size` (`release-small`) | x86_64 antes | x86_64 depois | aarch64 antes | aarch64 depois |
| --- | --- | --- | --- | --- |
| `text` | 148 215 | 160 535 (**+12 320**) | 133 792 | 144 924 (**+11 132**) |
| `rodata` | 43 229 | 45 144 (+1 915) | 38 981 | 40 864 (+1 883) |
| `data` | 19 850 | 20 962 (+1 112) | 19 624 | 20 736 (+1 112) |
| `bss` | 17 409 | 17 417 (+8) | 14 656 | 14 664 (+8) |
| **carregado** | 228 703 | 244 058 (**+15 355, +6,7%**) | 207 053 | 221 188 (**+14 135, +6,8%**) |
| arquivo | 310 256 | 332 736 (+22 480) | 345 128 | 371 576 (+26 448) |

Do que é do M1 (`llvm-nm -S`): a tabela de fins de processo ([DEBT-0133](../tech-debt/0133-fim-de-processo-numa-tabela-de-16-posicoes.md)) é `programs::ENDED`, 400 bytes de `.data` (16 posições de 24 bytes), e o `.bss` de +8 é o `WAITERS`. O resto do `.data` (+1 112 no total) e do `text` é o que o carregador, a paginação de usuário, os handles e a espera arrastam para a imagem; não separei por símbolo. As imagens `dev` e `release` já tinham tudo isso, então o M1 não as altera de forma que importe.

**Tempo de boot** (`make smoke-fast`, linha `boot time:`, 3 boots cada): **sem mudança**, porque o boot normal não lança processo; o carregador só roda quando o usuário digita `run`.

| ISA | Antes | Depois |
| --- | --- | --- |
| x86_64 (TCG) | 39, 39, 35 ms | 30, 29, 28 ms |
| aarch64 (HVF) | 4, 3, 3 ms | 2, 2, 2 ms |

A diferença é do ruído do host (o "antes" rodou com a máquina mais carregada: o x86_64 em TCG varia 24 a 40 ms entre execuções, como no [Tempo de boot](#tempo-de-boot)); nenhuma fase do boot ganhou linha nem chamada. `reclaimed:` não mudou (43 784 a 43 788 KiB no x86_64, 30 656 a 30 660 no aarch64).

**O initramfs cresce**: de cerca de 23 KB (só o `init`) para 93 412 bytes no x86_64 e 89 804 no aarch64, porque cada programa tem três a quatro páginas de segmento (um `PT_LOAD` por permissão, [SDD 17](17_processos_isolamento_e_syscalls.md#o-que-muda-no-linker-e-no-alvo)): `hello` 17 424, `crash` 17 296, `sysinfo` 17 648 e `count` 17 488 bytes no x86_64 (16 344 a 16 512 no aarch64). Esses frames ficam residentes ([DEBT-0080](../tech-debt/0080-initramfs-residente-sem-devolver-os-frames.md)): cerca de 17 frames (68 KiB) a mais de RAM em uso.

**Custo de lançar um processo, no shell**: o mesmo da 4b (o `init` consumia 21 frames: 7 de dados, 7 de tabelas e 7 do kernel, e 30 us até a entrada em usuário no aarch64); `ps` e `mem` voltam ao que eram quando o processo morre (a morte devolve os frames zerados). O boot normal segue com a pilha do shell em 13,0 KiB de 64 (`ps`, gate de 75%).

### A saída de segundo plano e o nome no `ps` (M1.1)

Medido em 2026-11-19, `make size` e `make smoke-fast PROFILE=release-small`, rustc 1.100.0, QEMU 11.1.3 (x86_64 em TCG, aarch64 em HVF, `-smp 4`, 256 MiB). "Antes" é a `main` `0ec2104`; "depois" é a mudança que arruma a saída de segundo plano que colava no prompt ([SDD 16](16_terminal.md#saída-de-segundo-plano)) e o nome `user` no `ps` (a fila de 1 KiB em `userout`, `Backlog`, `Shell::interject`, o nome de 16 bytes por tarefa).

| `make size` (`release-small`) | x86_64 antes | x86_64 depois | aarch64 antes | aarch64 depois |
| --- | --- | --- | --- | --- |
| `text` | 161 527 | 165 703 (+4 176) | 145 744 | 149 512 (+3 768) |
| `rodata` | 45 373 | 46 077 (+704) | 41 077 | 41 781 (+704) |
| `data` | 21 010 | 22 138 (+1 128) | 20 784 | 21 912 (+1 128) |
| `bss` | 17 417 | 18 481 (+1 064) | 14 664 | 15 720 (+1 056) |
| **carregado** | 245 327 | 252 399 (**+7 072, +2,9%**) | 222 269 | 228 925 (**+6 656, +3,0%**) |
| arquivo | 333 376 | 340 944 (+7 568) | 372 688 | 382 632 (+9 944) |

De onde vêm os bytes (`nm -S`): a fila de 1 024 bytes e o resto de `userout::GATE` são `.bss` (a fila sozinha é 1 KiB do +1 064: o estado inicial é tudo zero de propósito, para não ir ao `.data`); o `.data` de +1 128 é o nome de 16 bytes em cada slot de `task::TASKS` (24 slots, `Meta::user_name`); o `text` é o `Backlog`, o `interject`, o despejo da fila e a parte nova do laço do shell. O custo por processo em tempo não mudou: o nome é copiado uma vez por `spawn`.

**Tempo de boot** (`make smoke-fast`, linha `boot time:`, 3 boots cada): **sem mudança**, porque o boot não escreve saída de usuário nem lança processo.

| ISA | Antes | Depois |
| --- | --- | --- |
| x86_64 (TCG) | 37, 30, 32 ms | 28, 28, 30 ms |
| aarch64 (HVF) | 2, 2, 2 ms | 2, 2, 2 ms |

A diferença do x86_64 é o ruído do host (varia de 24 a 40 ms entre execuções, como no [Tempo de boot](#tempo-de-boot)). O caminho quente muda assim: cada `debug_write` pega mais um `IrqLock` (`userout::GATE`) e, na fila, sobe uma bandeira atômica e acorda o laço do shell; o laço pega o lock uma vez por byte digitado só se `pending()` (um `load`) diz que há o que mostrar.

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

### Processos, VMO e VMAR (fatia 4c, parte 3)

Medido em 2026-11-19, `make smoke` (modo de teste) e `make smoke-fast PROFILE=release-small`, rustc 1.100.0, QEMU 11.1.3 (x86_64 em TCG, aarch64 em HVF, `-smp 4`, 256 MiB). "Antes" é a `main` `0ec2104` (o M1), construída à parte com os mesmos comandos; "depois" é o PR. O TCG só compara o RustOS consigo mesmo.

| Medida | x86_64 (TCG) | aarch64 (HVF) |
| --- | --- | --- |
| Criar um processo (`process_create`, mapear o programa de 3 segmentos e a pilha, `thread_create`, o filho sair, esperar `TERMINATED`, soltar os handles), 300 em laço, metade morta no meio | 1,4 ms (de 1,4 a 13,5 ms: o TCG e as duas linhas de log serial por processo) | **141 µs** (de 141 a 605 µs) |
| Eco entre dois processos por canal com `object_wait_one`, ida e volta (o análogo do `lat_ctx`: duas trocas de processo), 2 000 voltas | 108 µs (de 108 a 116 µs) | **21,9 µs** (de 21,9 a 25,2 µs) |
| `process_kill` de processo bloqueado no kernel, até o `TERMINATED` | 1,4 ms | 86 µs |
| `process_kill` de processo com 4 threads girando em usuário (o IPI alcança até 3 CPUs de uma vez), até o `TERMINATED` | 1,4 a 2,7 ms | 38 a 44 µs |
| **Menor processo filho** (`child` esperando: frames que a criação gasta, de 14 a 17 no x86_64 e 14 a 15 no aarch64, e o heap) | 17 frames (68 KiB) e 9 548 B | 15 frames (60 KiB) e 9 548 B |
| Falta de página, primeiro toque / leitura da página zero / escrita sobre ela (o lock por espaço entrou; ns por página) | 15 145 / 4 192 / 7 237 (antes 14 414 / 3 709 / 6 740) | 640 / 247 / 760 (antes 750 / 251 / 1 104) |
| Syscall nula, pilha limpa / sem limpar (ns) | 675 / 274 (antes 540 / 240) | 63,9 / 52,6 (antes 60,8 / 48,9) |
| `make size` `release-small`, carregado (bytes) | 263 831 (antes 245 327: **+18 504, +7,5%**) | 238 141 (antes 222 269: **+15 872, +7,1%**) |
| `boot time:` do `smoke-fast` `release-small` (3 boots) | 29, 32 e 36 ms (antes 32 e 32: o TCG oscila) | 2, 2 e 3 ms (antes 2 e 2) |

- **O menor filho: 15 a 17 frames mais 9,5 KB de heap.** O `init`, carregado pelo kernel, gasta 24 a 25 (11 de dados, 7 de tabelas, 7 do kernel). Os frames do filho são os de VMO que alguém tocou, as tabelas de página, a pilha de kernel e a área de FPU; a conta é medida (frames livres antes e depois de o filho nascer e ficar bloqueado), **não** somada por origem, e inclui o que o heap do kernel cresce. Os 8 KiB cobrados ao pai por processo e os 4 KiB por thread (12 KiB) cobrem os 9,5 KB de heap com folga de 25%.
- **O teto em bytes:** o `procs` esgota as mensagens em 10 pares e 311 a 322 KB (o teto é de 512 KiB, e cada ponta custa 4 096 mais o payload e 64 por mensagem), os VMOs de uma página em 1 590 a 1 637 e os processos que nunca rodam em 25 (8 KiB cada, 25 × 8 KiB = 200 KiB, mais o resto que o programa já guardava). Depois de fechar tudo a conta volta e a criação segue.
- **Nada vaza:** de `A` (depois do aquecimento) a `C` (depois de 300 processos, 2 000 ecos e um filho bloqueado) o kernel confere **0 frames, 0 B de heap e 0 páginas prometidas** de diferença, nas duas ISAs. O mesmo confere o smoke do M1 (o heap igual depois de 20 `run hello`).
- **O custo da parada de thread na syscall nula:** a conferência de "o processo morre?" no fim de `dispatch` custou 3 vezes a syscall nula no TCG enquanto era uma busca pelo processo (1 650 ns com a pilha limpa, contra 540). O bit `kill` do slot da tarefa, lido com leitura do bloco da CPU e do slot **embutidas** (`inline(always)`, que o clippy pede para justificar), levou a 675 ns: o que sobra, +25% no TCG e +5% no HVF, é a leitura. **Inferência, não conferida:** a causa dos 3 vezes seria o quadro a mais da chamada, que empurra a sujeira da pilha para além da janela de 256 bytes que o `arch/` repinta sem custo, e a repintura em Rust é cara no TCG; o que foi medido é só que embutir as leituras recuperou a maior parte.
- **A falta de página com o lock por espaço** (a medida que a dívida 0120 pedia): no HVF não há diferença visível (o ruído é de 15%); no TCG, 5 a 13%. O custo do lock sem disputa é uma troca de máscara de IRQ e um `compare_exchange`.
- **Tamanho:** +18,5 KB no x86_64 e +15,9 KB no aarch64 vêm das 7 chamadas novas, do `Mem`, do VMO, da cota, da morte com threads e do `ThreadObj`: quase tudo está no `text` (+18 608 e +15 828 bytes), 0,5 KB no `rodata` e 0,66 KB no `data`; o `bss` não mudou.
- **Tempo de boot:** sem mudança, o boot normal não cria processo (o `init` segue só no modo de teste, [DEBT-0087](../tech-debt/0087-init-so-no-boot-de-teste.md)); a diferença do x86_64 é a oscilação do TCG ([DEBT-0068](../tech-debt/0068-boot-de-teste-do-x86-em-tcg-bimodal-por-layout.md)).
- **O que não foi medido:** a latência do eco em KVM ou metal, o custo da criação com PCID/ASID (4e) e com o heap sob disputa em N CPUs ([DEBT-0054](../tech-debt/0054-heap-com-lock-global.md) disparou com a criação de processos em várias CPUs: o `procs` cria um por vez, e a disputa não foi medida).

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

### Imagem enxuta: sem o modo de teste

Medido em 2026-11-18, `make size` e `llvm-nm --size-sort` (do componente `llvm-tools` do `rust-toolchain.toml`: o `cargo bloat` é crate externo), rustc 1.100.0, na `main` `b495b0c` (antes) e nesta mudança (depois). O modo de teste ([SDD 04](04_boot.md#modos-de-boot)) ia em toda imagem, inclusive na `release-small`, a de produção. Agora é a feature `selftest` ([SDD 05](05_build_e_execucao.md#perfis)): `dev` e `release` a levam, a `release-small` não. Cada linha abaixo é `carregado / arquivo` em bytes; **negrito** é o que o perfil monta por padrão.

| ISA | Perfil | Antes | Depois, com `selftest` | Depois, sem `selftest` | Ganho do que o perfil monta (carregado) |
| --- | --- | --- | --- | --- | --- |
| x86_64 | `dev` | 381 498 / 5 428 968 | **381 562 / 5 464 600** | 322 494 / 4 741 640 | +64 (com a feature; sem ela, −59 004, −15,5%) |
| x86_64 | `release` | 346 760 / 388 184 | **346 448 / 388 240** | 283 612 / 319 392 | −312 (sem ela, −63 148, −18,2%) |
| x86_64 | `release-small` | 210 692 / 286 080 | 210 620 / 286 072 | **183 556 / 251 248** | **−27 136 (−12,9%)**; arquivo −34 832 (−12,2%) |
| aarch64 | `dev` | 311 821 / 5 257 616 | **310 333 / 5 299 208** | 265 861 / 4 604 416 | −1 488 (sem ela, −45 960, −14,7%) |
| aarch64 | `release` | 292 633 / 363 152 | **292 309 / 359 104** | 240 641 / 300 864 | −324 (sem ela, −51 992, −17,8%) |
| aarch64 | `release-small` | 189 501 / 320 024 | 189 489 / 319 968 | **166 665 / 279 368** | **−22 836 (−12,1%)**; arquivo −40 656 (−12,7%) |

A imagem de produção cai de 205,7 para 179,2 KiB (x86_64) e de 185,0 para 162,7 KiB (aarch64). Ligar a feature não custa nada: a `release-small` com `selftest` tem 210 620 e 189 489 bytes, dentro de 100 bytes da `main`. O `.bss` cai 248 bytes (x86_64) e 200 (aarch64), das estáticas das medidas. O boot normal não muda: 25 ms no x86_64 (TCG) e 2 ms no aarch64 (HVF), antes e depois, porque o modo normal já não rodava o que saiu.

**O que saiu, em duas etapas** (`release-small`, x86_64 / aarch64): o código do kernel, 187 620 / 170 221 bytes carregados (−23 072 / −19 280); depois o comando `bench` do `core/shell`, atrás da feature `bench` do crate (−4 536 / −3 692), para 183 084 / 166 529. Depois, ao mover o código de teste de `task` e `frames` para submódulos e deixar `FpuSummary`, `boot_time::Summary` e `VmStats` sempre completos (menos `cfg` espalhado), a imagem enxuta ficou 472 / 136 bytes maior: 183 556 / 166 665. Os números do `nm` abaixo são da etapa anterior. O `nm` do que só existe com a feature soma 20 246 (x86_64) e 17 669 bytes (aarch64) em símbolos próprios, e o resto da diferença está no que o compilador tinha embutido em funções que ficam (`boot_kernel` encolhe 4 884 / 3 752 bytes, `report_boot_time` 592 / 196) e em `.rodata` (−3 800 / −3 416 bytes, cadeias de formatação das linhas de medida). Os maiores, no x86_64 `release-small`:

| Símbolo | Bytes | O que é |
| --- | --- | --- |
| `selftest::before_tasks` | 2 951 | heap, resíduo de chave, timer, `random bench:` e `zero bench:`, com o que o compilador embutiu |
| `bench::run` | 2 517 | as medidas de tarefa: troca, IPI, escala, tempestade, jitter |
| `shell::bench::run` | 2 436 | o comando `bench` do shell |
| `Rng::fill_bytes` | 1 120 | o gerador em bloco; só existe como símbolo na imagem com a feature |
| `heap::selftest::zero_benchmark` | 863 | a medida de zerar |
| `bench::storm_ns` | 712 | a tempestade de alocação |
| `random::selftest::stack_residue` | 668 | a varredura de resíduo de chave |

**Fica de propósito**: o autoteste de frame, o `w^x:` e o de exceção ([SDD 04](04_boot.md#modos-de-boot)), e a pintura das pilhas que o `ps` mostra. **Não foi medido**: o que o `lto = "fat"` ainda guarda do modo de teste em funções compartilhadas (por exemplo `Frames::allocate`, que ficou 132 bytes menor sem a feature), nem a diferença em hardware real.

**Memória no boot normal, com e sem a feature** (`release-small`, `-smp 4`, `make smoke-fast` e `mem`, 3 boots cada; os frames são exatos, de uma linha `frames:` temporária no `report` que não está no kernel, porque o modo normal não a imprime):

| ISA | Imagem | `kernel` do `mem` | `reclaimed:` | Frames livres | Usável | `mem`: livre / em uso |
| --- | --- | --- | --- | --- | --- | --- |
| x86_64 (TCG) | com `selftest` | 216,0 KiB | 43 716 a 43 720 KiB | 63 646 a 63 647 | 255 444 KiB | 248,5 MiB / 888 KiB |
| x86_64 (TCG) | sem | 192,0 KiB | 43 680 a 43 684 KiB | 63 652 | 255 468 KiB | 248,6 MiB / 888 a 896 KiB |
| aarch64 (HVF) | com `selftest` | 196,0 KiB | 30 588 KiB | 63 198 | 253 160 KiB | 246,8 MiB / 400 KiB |
| aarch64 (HVF) | sem | 172,0 KiB | 30 548 a 30 552 KiB | 63 202 a 63 203 | 253 184 KiB | 246,8 MiB / 404 KiB |

(Os frames exatos são da etapa anterior à mudança de `cfg` acima; o `kernel` do x86_64 sem a feature subiu de 188 para 192 KiB por arredondamento de segmento, e a memória livre do `mem` seguiu em 248,6 MiB.) A região da imagem encolhe 24 a 28 KiB, mas a memória livre cresce só de 5 frames (20 KiB) no x86_64 e de 4 a 5 (16 a 20 KiB) no aarch64: 0,008% dos 256 MiB, abaixo da resolução do `mem` (0,1 MiB). O ganho é do tamanho da imagem, que é o que pesa em hardware pequeno (disco, flash, carga), não da memória ociosa.

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
  `-machine virt,gic-version=3 -cpu max -m 256M -accel hvf` (aarch64, com `-boot menu=on,splash-time=0`), `-nic none`,
  `-no-reboot`, `-display none -serial stdio -monitor none`, mais `-smp N`. O RustOS sobe com
  `-smp 4` (o padrão do `make`; `RUSTOS_SMP=N` troca, [SDD 05](05_build_e_execucao.md)); o Linux foi medido com 1 e com 4, e o RustOS com 4 (a medida atual) e com 1.
- **Pelo EDK2, como o RustOS:** `-bios edk2-aarch64-code.fd -boot menu=on,splash-time=0` (aarch64) e `-drive
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

**RustOS** (`perf/lean-image` sobre a `main` `b495b0c`, `release-small` **sem a feature `selftest`** (a imagem de produção, [acima](#imagem-enxuta-sem-o-modo-de-teste)), `-smp 4` (o padrão do `make`), pilhas de 16 KiB, 2026-11-18; `make smoke-fast` para o `reclaimed:` e `make screenshot ... CMDS="mem"` para o `mem`, 3 boots cada, QEMU 11.1.3, o mesmo host). O modo normal não imprime `memory:` nem `frames:`: o disponível sai do `mem`.

| ISA | `-smp` | `reclaimed:` (3 boots) | `mem`: livre (3 boots) | em uso (`mem`) | reservado (`mem`) | Disponível (livre + cache devolvível) | Usado: 256 − disponível |
| --- | --- | --- | --- | --- | --- | --- | --- |
| x86_64 (TCG) | 4 | 43 680 / 43 680 / 43 680 KiB | 248,6 / 248,6 / 248,6 MiB | 892 / 892 / 896 KiB | 2,1 MiB | **248,6 MiB** | **7,4 MiB** |
| aarch64 (HVF) | 4 | 30 548 / 30 548 / 30 548 KiB | 246,8 / 246,8 / 246,8 MiB | 404 / 404 / 404 KiB | 12 KiB | **246,8 MiB** | **9,2 MiB** |
| x86_64 (TCG) | 1 | 1 boot | 248,9 MiB | 692 KiB | 2,0 MiB | 248,9 MiB | 7,1 MiB |
| aarch64 (HVF) | 1 | 1 boot | 246,9 MiB | 296 KiB | 12 KiB | 246,9 MiB | 9,1 MiB |

O cache de página do RustOS não existe ainda, então o disponível é o livre do buddy. Dos
7,4 e 9,2 MiB "usados", só 0,9 e 0,4 MiB são memória em uso dentro do mapa usável; o resto
(6,5 e 8,8 MiB) é o que o firmware e o bootloader não entregam como RAM, como antes. O mesmo buraco existe do lado
do Linux (ver acima), e a imagem do kernel do RustOS está nesse resto, não nos 0,9 e 0,4 MiB "em uso". Contra a medida anterior (`main` `24be32a`, 1 CPU, `release`: 6,9 e 9,1 MiB, em uso 380 e 320 KiB), com a mesma 1 CPU: **x86_64 +0,2 MiB** (em uso 380 para 692 KiB, dos quais cerca de 316 KiB são o histórico de rolagem de 256 linhas, 158 colunas × 8 bytes, que veio depois, [SDD 16](16_terminal.md)) e **aarch64 0,0** (em uso 320 para 296 KiB; o aarch64 da `virt` não aloca histórico). Os núcleos a mais custam 200 KiB (x86_64: 692 para 892 KiB) e 108 KiB (aarch64: 296 para 404 KiB) para 3 CPUs, 67 e 36 KiB por CPU, com a pilha de 16 KiB, as pilhas de exceção e a política no heap ([SDD 15](15_tarefas_e_preempcao.md#custo-por-tarefa-e-por-cpu)); não os decompus por item.

**Razão contra o Linux de distro com raiz mínima, com o mesmo `-smp`** (as mesmas ressalvas da seção; Linux: mediana do `MemAvailable`, 3 boots, tabela acima):

| ISA | `-smp` | Linux mínimo: usado | RustOS (sem `selftest`): usado | RustOS / Linux | Linux gasta |
| --- | --- | --- | --- | --- | --- |
| x86_64 | 4 | 70,9 MiB | 7,4 MiB | **10,4%** | 9,6x |
| aarch64 | 4 | 70,0 MiB | 9,2 MiB | **13,1%** | 7,6x |
| x86_64 | 1 | 72,2 MiB | 7,1 MiB | 9,8% | 10,2x |
| aarch64 | 1 | 61,7 MiB | 9,1 MiB | 14,7% | 6,8x |

Antes (`24be32a`, RustOS com 1 CPU contra o Linux com 1): 9,6% no x86_64 (6,9 contra 72,2) e 14,7% no aarch64 (9,1 contra 61,7). Com 4 CPUs dos dois lados a razão sobe no x86_64 (10,4%) e cai no aarch64 (13,1%): o Linux gasta 5 a 8 MiB a mais com 4 CPUs no aarch64 (`Slab`, pilhas, dados por CPU) e nada no x86_64, e o RustOS gasta 0,3 (x86_64) e 0,1 MiB (aarch64) a mais. As ressalvas são as da seção: o Linux é a raiz mínima, e o `MemAvailable` já desconta de 2,5 a 3 MiB que o RustOS não desconta.

**O que cada lado tem**

- **Linux:** kernel `virt` de distro (a linha `Memory:` do `dmesg` soma code, rwdata, rodata, `init` e
  `bss`: 30 MiB no x86_64 e 34 no aarch64; o `init`, de 2 a 3 MiB, é liberado depois), `struct page` por frame, caches do slab (7 a 12 MiB), um BusyBox e a raiz em `tmpfs`.
  Sem módulos carregados, sem rede (`-nic none`), sem disco montado.
- **RustOS:** kernel carregado de 179,2 KiB (x86_64) e 162,7 KiB (aarch64) (`make size PROFILE=release-small`, sem `selftest`: 183 556 e 166 665 bytes; o `mem` mostra 192 e 172 KiB na linha `kernel`, com o arredondamento por segmento), shell embutido no kernel, console na
  tela e na serial, buddy, heap, escalonador, tarefas. Sem espaço de usuário, sem cache de
  página, sem drivers de disco ou de rede.

**O que a diferença não prova**

- Que o RustOS final vai gastar de 10 a 15% do que a base gasta (hoje, com `-smp 4` dos dois lados, 7,4 contra 70,9 MiB no x86_64 e 9,2
  contra 70,0 no aarch64). Hoje ele não tem nada que custe memória por processo, por arquivo ou por
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
(`Slab`, pilhas e dados por CPU); no x86_64 não houve aumento. O alvo acima foi escrito com a base de `-smp 1` (36 e 30,8 MiB); com a base de `-smp 4`, que é o que o RustOS roda hoje, a metade seria 35,5 (x86_64) e 35,0 MiB (aarch64). **Não mudei o alvo**: trocar a base dele é decisão do dono.

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

## Tempo de boot

Medido em 2026-11-18, `release`, QEMU 11.1.3 no macOS 26.4 (Apple M5), 256 MiB, `-smp 4`, `-nic none`, firmware EDK2 do pacote da QEMU, Limine 12.10.0. As flags são as de `xtask/src/arch.rs`; a aceleração é a do SDD 05: **TCG no x86_64, HVF no aarch64**. Cada número é a mediana de 3 boots (Linux) ou de 5 (RustOS), de processos novos da QEMU. O host dividia a máquina com outras cargas (carga média de 5 a 9 durante as medidas do RustOS): em silêncio o modo normal do x86_64 deu 24 ms, e as medianas abaixo são as de 5 boots sob carga.

### Como se mede

- **RustOS, kernel:** a linha `[rustos] boot time: kernel X ms to prompt (memory A, smp B, self-tests C, benches D)`. O relógio de `kernel/src/boot_time.rs` conta do `kmain` (depois do Limine) até o `boot ok`, que vem logo antes do prompt, em ciclos do contador, e converte no fim. No modo de teste, `boot phases:` dá o que a linha não mostra. `memory` soma as fases de memória e de `reclaim`.
- **RustOS, firmware e Limine:** a linha `boot timeline:` (pedido *bootloader performance* do Limine): do reset ao Limine e do Limine ao kernel.
- **RustOS, de parede:** do `spawn` da QEMU ao `boot ok` na serial (`make smoke`), ou ao prompt do shell (`rustos ~ % `) no script de comparação.
- **Linux:** Alpine 3.24.4, kernel `6.18.60-0-virt`, com a **raiz mínima** (só o `busybox`) do SDD 07 acima, `-append "console=… rdinit=/bin/sh"`. Kernel sozinho: o carimbo de tempo do `printk` em `Run /bin/sh as init process` (o relógio dele começa no `start_kernel`; o carregamento do `bzImage` e o *stub* EFI ficam de fora, como o Limine fica de fora do RustOS). De parede: do `spawn` da QEMU até o shell **responder** a um `echo` digitado na serial.
- **Os dois pelo EDK2** (`-bios` / `pflash`; no aarch64, os dois com `-boot menu=on,splash-time=0`), como o RustOS; o Linux entra pelo `-kernel`, que o EDK2 entrega pelo `fw_cfg`. Também sem firmware (`-kernel` direto, sem EDK2), para isolar o que o EDK2 custa.

Os scripts de medida não estão no repositório (viram `xtask` quando a comparação passar a rodar no CI); o método acima os reproduz com o `time.monotonic()` do host em volta de cada leitura da serial.

### Antes e depois, por fase (kernel, ms)

"Antes" é a `main` `5fe47b4` só com o relógio por fase. "Depois, teste" é o `make smoke` (modo de teste); "depois, normal" é o `make smoke-fast`. O x86_64 em TCG é número do RustOS contra ele mesmo (guidelines 05).

| Fase | x86_64 antes | x86_64 teste | x86_64 normal | aarch64 antes | aarch64 teste | aarch64 normal |
| --- | --- | --- | --- | --- | --- | --- |
| Memória e VM (`memory`, com o `reclaim`) | 4 | 5 | 5 | 0,0 | 0,0 | 0,0 |
| Console (serial e tela) | 3,5 | 3,6 | ~3,6 | 0,1 | 0,1 | ~0,1 |
| Interrupções e calibração do timer | 21,3 | 6,9 | ~6,9 | 0,1 | 0,1 | ~0,1 |
| SMP (`wait_online`) | 1 | 1 | 1 | 0,0 | 0,0 | 0,0 |
| Estado: gerador, linhas, `task::init` (`setup`) | 32,0 | 17,3 | ~10 | 4,7 | 3,4 | ~1 |
| Autotestes (`self-tests`) | 346 | 339 | 3 | 338 | 332 | 1 |
| Medidas (`benches`) | 553 | 328 a 3 389 (depende do binário) | 0 | 110 | 116 | 0 |
| **Kernel até o prompt** | **964** | **707 a 3 774** (mediana 2 965) | **30** (24 em silêncio) | **453** | **450** | **2** |
| Parede até o `boot ok` | 2 700 | 4 857 | 1 915 | 5 650 | 5 619 | 5 169 |
| Do qual firmware + Limine (`boot timeline:`) | 1 650 | 1 850 | 1 850 | 5 130 | 5 130 | 5 120 (**124** com a flag; parede 172) |

As colunas "normal" têm `setup` e `console` derivados: o modo normal não imprime `boot phases:` (cada linha custa tempo, [SDD 04](04_boot.md#modos-de-boot)), e eles são o total menos o que a linha `boot time:` mostra.

**O tempo das medidas do modo de teste do x86_64 em TCG depende do binário**: as mesmas medidas curtas levam de 0,3 a 3,4 s conforme o tamanho e o alinhamento do código (a revisão do PR provou com preenchimento de `nop`: a `main` com 2 500 a 5 000 bytes a mais fica lenta, este PR com 3 000 a mais fica rápido, mais rápido que a `main`). Cada binário fica quase sempre num modo; não é trabalho novo (`self-tests` 339 ms é a parte estável). Por isso o `make smoke` do x86_64 subiu de ~2,7 para ~4,5 s de parede ([DEBT-0068](../tech-debt/0068-boot-de-teste-do-x86-em-tcg-bimodal-por-layout.md)).

### O que cada bloco custava no boot de antes

Diferenças de tempo entre linhas consecutivas da serial, `release`, um boot (a mediana de 5 do total está acima):

| Bloco | x86_64 (TCG) | aarch64 (HVF) | Hoje |
| --- | --- | --- | --- |
| `timer:` (5 tiques a 100 Hz) | 51,6 ms | 49 ms | só no modo de teste |
| `idle:` (janela de 50 ms) | 52,5 ms | 51,4 ms | só no modo de teste |
| `task self-test:` (preempção e SIMD) | 83,5 ms | 81 ms | só no modo de teste |
| `smp self-test:` (migração) | 142,9 ms | 141,8 ms | só no modo de teste |
| `switch:` + `scale:` + `alloc storm:` (esforço `BOOT`) | 483 ms | 106,8 ms | só no modo de teste |
| `zero bench:` | 51,7 ms | 1,7 ms | só no modo de teste |
| `random bench:` | 13,1 ms | 2,5 ms | só no modo de teste |
| `heap self-test:` | 11,6 ms | 0,4 ms | só no modo de teste |
| `key residue:` | 1,4 ms | 0,55 ms | só no modo de teste |
| `frame self-test:` | 1,9 ms | 0,6 ms | sempre |
| `w^x:` | 1,8 ms | 0,56 ms | sempre |
| `exception self-test:` | 0,9 ms | 0,3 ms | sempre |
| Calibração do PIT (x86_64) | 20 ms | não há | 5 ms (uma janela que fecha por incerteza, SDD 10) |
| Primeiro `render` da tela cheia (x86_64) | ~15 ms | não medido | 0 |
| Uma linha na serial, com tela | 1,0 ms | 0,45 ms | menos linhas no modo normal |
| Uma linha na serial, sem tela | 0,3 ms | 0,38 ms | |
| `smp::start` + `wait_online` | 1 ms | < 0,1 ms | igual |
| `reclaim_bootloader` | 0,3 ms | 2 µs | igual |
| Zerar memória grande no boot | não há | não há | `Memory::init` leva 0,9 ms e o `Vm::build` 2,2 ms no x86_64; nenhum zera mais que a grade da tela |

O que se mexeu, no x86_64 em TCG, com o kernel do modo normal medido depois de cada passo (um boot cada, host em silêncio; ms): autotestes e medidas atrás de `rustos.selftest=1`, 965 → 65; calibração do PIT em 5 ms e o primeiro `render` sem repintar a grade vazia, juntos, 65 → 38 (o PIT custa 15 por conta e o `render` inicial, ~15 observado na primeira linha impressa; a calibração final (uma janela que fecha por incerteza) deu mediana de 26 ms de kernel em 12 boots, de 26 a 31, [SDD 10](10_interrupcoes.md#calibração-pelo-pit-uma-janela-que-cresce-até-fechar)); menos linhas no boot normal e o `render` que junta células sem tinta, 38 → 26. O `wait_online` do SMP, o `reclaim` e a "memória grande" não estavam no caminho: 1 ms ou menos nas duas ISAs. A calibração do PIT e o redesenho só pesam em TCG; no aarch64 o boot normal já é 2 ms, e o que sobra é serial (cada byte escrito na PL011 é uma saída da VM) e `task::init`.

### Comparação com o Linux (mesma QEMU)

Três boots, mediana. O RustOS está no modo normal; "antes do kernel" é o de parede menos o kernel sozinho.

| ISA (aceleração) | Sistema | Parede até o shell | Antes do kernel | Kernel sozinho |
| --- | --- | --- | --- | --- |
| x86_64 (TCG) | **RustOS**, pelo EDK2 e o Limine | **1,67 s** | 1,64 s (EDK2 1,34 s, Limine 0,25 s) | **24 ms** |
| x86_64 (TCG) | Linux, pelo EDK2 (`-kernel`) | 3,71 s | 2,18 s (EDK2 e o *stub* EFI) | 1,54 s |
| x86_64 (TCG) | Linux, sem firmware (`-kernel` direto) | 2,33 s | 0,94 s (QEMU, SeaBIOS e o `bzImage`) | 1,39 s |
| aarch64 (HVF) | **RustOS**, pelo EDK2 e o Limine | **0,17 s** (5,24 s sem a flag) | 0,17 s (EDK2 0,10 s, Limine 0,02 s) | **2 ms** |
| aarch64 (HVF) | Linux, pelo EDK2 (`-kernel`) | 0,34 s | 0,27 s (EDK2 e o *stub* EFI) | 69 ms |
| aarch64 (HVF) | Linux, sem firmware (`-kernel` direto) | 0,18 s | 0,11 s | 66 ms |

**O EDK2 não pesava igual nos dois no aarch64, e a causa era a espera de 5 s do gerenciador de boot** (`PcdPlatformBootTimeOut`; fontes e flag no [SDD 05](05_build_e_execucao.md#qemu-e-firmware)). A QEMU só a zera com `-boot menu=on,splash-time=0`, e o `-kernel` do Linux já a pulava (0,3 s). Medido antes (HVF, `firmware` de 5,1 s): os mesmos 5,1 a 5,7 s em TCG, então não é do HVF; sem disco, sem `ramfb`, ou com NVRAM vazia em pflash, os mesmos 5,2 s; uma tecla na serial durante a espera não adianta; `ramfb`, `bootindex`, `-boot menu=off`, tipo de FAT e disco só leitura não mudam. Com a flag o `firmware` cai para 0,10 s (mediana de 5: 96 a 103 ms; TCG 0,47 s) e o Limine leva 0,02 s. **Mesma configuração nos dois lados** (`-bios` e `-boot menu=on,splash-time=0`): o RustOS leva 0,17 s de parede (host com carga média de 9) e o Linux pelo EDK2 e `-kernel`, com a raiz mínima, 0,34 s (mediana de 6 boots; sem a flag 0,33 s: o `-kernel` não esperava). O RustOS passa a ganhar na parede, com a ressalva de que ainda não tem drivers nem espaço de usuário (abaixo). No x86_64 o caminho de disco do EDK2 (1,32 s) já era mais curto que o `-kernel` do Linux com o *stub* (2,18 s). O Linux não poderia ser entregue pelo Limine para igualar o caminho no aarch64: o `vmlinuz-virt` do Alpine é um PE EFI comprimido, não um `Image`.

**Ressalvas, honestas:**

- **O RustOS ainda não tem drivers nem espaço de usuário.** O Linux, até `Run /bin/sh`, inicializa o *subsistema de dispositivos* (virtio, PCI, serial, rede, o `rootfs`), descomprime o initramfs e executa o `/bin/sh`; o RustOS chega a um shell que roda dentro do kernel, sem disco, rede nem processo. A comparação mede "boot até um prompt", não "boot até a mesma capacidade".
- **Número de TCG não é latência** (guidelines 05): no x86_64 os dois são TCG, então a razão (1,5 s contra 24 ms) vale mais que os valores, e mesmo ela mistura o custo do TCG por instrução (o Linux executa muito mais). O número de latência é o do aarch64 com HVF: 69 ms contra 2 ms no kernel sozinho.
- **O `printk` do Linux não conta o que vem antes do `start_kernel`**; o do RustOS não conta o Limine. É o mesmo corte (o primeiro código do kernel), mas o Linux faz parte da inicialização da CPU e da memória antes dele (o *stub* EFI e a descompressão), que entram em "antes do kernel".
- **O Linux é o de distro (`virt`, 13 MiB)**, não um kernel enxuto: o `tinyconfig` não foi medido (SDD 08).
- Três boots por célula, host compartilhado: a dispersão do x86_64 em TCG é de ±3% entre boots em silêncio e maior sob carga.

## Método de comparação com Linux e Windows

1. **Mesma máquina, mesmo firmware, versões anotadas.** Firmware e ACPI mudam o
   tempo de boot; cada rodada registra host, QEMU, firmware e versão do sistema
   ([SUSE, KVM Forum 2022](https://gitlab.com/qemu-project/kvm-forum/-/raw/main/_attachments/2022/QEMU%20%26%20KVM%20Automated%20Performance%20Benchmarking%20%40%20SUSE-1.pdf)).
2. **QEMU para função, metal para latência.** Sob TCG o mesmo boot de Linux levou
   ~8,25 s contra ~1,22 s com KVM
   ([Santilli](https://cirosantilli.com/linux-kernel-module-cheat/benchmark-linux-kernel-boot.html)):
   número de latência só vale com aceleração de hardware ou em metal.
3. **Boot medido até a primeira instrução de usuário**, não até um shell. Enquanto não houver espaço de usuário no RustOS, a comparação é até o prompt dele contra `Run /bin/sh` no Linux, com a ressalva da seção [Tempo de boot](#tempo-de-boot).
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
