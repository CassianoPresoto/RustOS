# SDD 15 — Tarefas, troca de contexto e preempção

> **O que este capítulo responde:** como o kernel cria, escolhe e troca threads, como
> o timer preempte uma tarefa em cada ISA, onde fica o estado de FPU/SIMD e que regra
> de lock a troca exige.

Fase 3 do [roadmap](08_roadmap.md), com todos os núcleos (SMP). A política de escolha é o
`Composite` (FIFO e EEVDF) de `core/sched`, atrás do trait `Scheduler` ([SDD 14](14_escalonador.md)),
**uma instância por CPU**; este capítulo é a cola do kernel. Tempo e timer estão no
[SDD 10](10_interrupcoes.md); pilhas, no [SDD 13](13_memoria_virtual.md); como os núcleos
sobem, no [SDD 04](04_boot.md).

## Quem é quem

| Peça | Onde | Papel |
| --- | --- | --- |
| `Context`, `switch` | `arch/<isa>/src/context.rs` | Salva os registradores callee-saved e o `sp`; `Context::new` monta a thread nova (trampolim `rustos_task_start`) |
| `fpu` | `arch/<isa>/src/fpu.rs` | Liga FPU/SIMD, mede a área de estado, `save` e `restore` |
| `xstate` | `arch/x86_64/src/xstate.rs` | Puro, testado no host: `XCR0` a ligar, tamanho da área, estado inicial |
| `hal::Handlers.preempt` | `core/hal/src/interrupt.rs` | Gancho que o `arch/` chama no fim de toda IRQ, depois do EOI |
| `percpu.rs` | `kernel/src/percpu.rs` | `PerCpu`: um bloco por núcleo, achado por `GS`/`TPIDR_EL1` (porta `hal::CpuLocal`): índice, `hw_id`, `in_irq`, `locks_held`, prazo armado, carga, a fila e os contadores |
| `smp.rs` | `kernel/src/smp.rs` | Solta os núcleos secundários (pedido MP do Limine) e leva cada um do `goto_address` até a ociosa |
| `task.rs` | `kernel/src/task.rs` | Tabela de tarefas, a fila por CPU (`Queue`), `spawn`, `yield_now`, `sleep_until`, `park`/`unpark`, `exit`, a troca, a migração, o IPI de reescalonar e a preempção |
| `panic.rs` | `kernel/src/panic.rs` | O pânico de um núcleo para os outros por IPI e toma o console |
| `interrupts.rs` | `kernel/src/interrupts.rs` | IRQ do timer: o que o escalonador pede (acordar, fim de fatia) e o rearme do disparo |
| `bench.rs` | `kernel/src/bench.rs` | Autoteste de SIMD, latência de troca e jitter de acordar |
| `ps`, `bench` | `core/shell`, porta `Host` (`tasks`, `bench`) | Os dois comandos do shell |

`hal` não ganhou porta de contexto: nenhum crate de `core/` troca de contexto, e quem
a usa é só o `kernel/`, pelo `platform`. `core/sched` só decide.

## A tarefa

Cada slot de `TASKS` (`MAX_TASKS` = 32: 8 ociosas e 24 tarefas) guarda, em atômicos que qualquer
CPU lê sem lock, o estado, a CPU dona (`cpu`), `on_cpu`, o aviso de `unpark` adiantado (`permit`), a
máscara de afinidade, o prazo de acordar, o tempo de CPU em ns, quantas vezes foi escolhida e o
endereço da área de FPU/SIMD; e, atrás de um `IrqLock` por slot, o que só muda no `spawn`: nome,
função, argumento, topo da pilha e a área de FPU. Os parâmetros de escalonamento (`SchedParams`) vão para
a política da fila em `spawn`. O `Context` fica fora de qualquer lock, em `CONTEXTS`: a troca o usa com o lock
da fila já solto. `TaskId` é o número do slot.

| Estado | Quando |
| --- | --- |
| `Free`, `Exited` | Slot disponível para `spawn` (a pilha mapeada e a área de FPU ficam com o slot). `Exited` só vale depois que a CPU dele terminou a troca: `on_cpu` baixo |
| `New` | Reservado por `spawn` (um `compare_exchange` de `Free` ou `Exited` para `New`), sem contexto ainda |
| `Ready`, `Running` | Pronta, numa fila; ou a que ocupa uma CPU |
| `Blocked` | Em `park`, até um `unpark` (ou o aviso já estava lá) |
| `Sleeping` | Em `sleep_until`, até o prazo |

- **Slots `0..MAX_CPUS` são as ociosas**, uma por CPU, fora da política (como o `idle` do Linux):
  rodam quando `pick_next` devolve `None` e fazem `hlt`/`wfi` com a interrupção ligada. A da CPU de boot
  é uma thread de pilha própria (4 páginas); a de cada núcleo secundário **é a execução que subiu**,
  na pilha de 16 KiB que o `smp::start` mapeou.
- **O slot `MAX_CPUS` é o boot.** `task::init` adota a execução do `kmain`, com a pilha de 64 KiB
  que já tinha, como tarefa `init`. Ela roda os autotestes e, em `run_shell`, vira a
  tarefa `shell`, classe justa, nice 0 e fatia de 200 µs (interativa: o prazo virtual
  curto a põe na frente de quem só calcula, e `PREEMPT_SHORT` fura a proteção da fatia
  de uma corrente de 700 µs). O shell dorme em `park` quando a fila de entrada está vazia, e a IRQ
  da serial ou do teclado (na CPU de boot, [DEBT-0056](../tech-debt/0056-irqs-de-dispositivo-so-na-cpu-de-boot.md)) o acorda (`task::notify_input`), em qualquer CPU em que ele esteja. `wait_for_input` registra o waiter e **confere a fila de novo** antes de dormir (`SeqCst` dos dois lados): a IRQ de outra CPU pode empilhar o byte entre a conferência do shell e o registro, o que com uma CPU só a máscara impedia. O `flush` do terminal roda antes, sob a mesma máscara (SDD 16).
- **Cada tarefa nova tem pilha própria:** 4 páginas (16 KiB, `task::STACK_PAGES`) e uma de guarda sem
  mapeamento, na janela de pilhas (`memory::map_stack`). Reusar o slot reusa a pilha:
  a janela é só de avanço. O tamanho vem da medida de pico ([Pico de pilha](#pico-de-pilha)).
- **`spawn` com os slots cheios** falha com `NoSlot` mesmo se um deles é `Exited` com `on_cpu` ainda alto (a CPU dele não terminou a troca): a janela é de uma troca, e quem cria tarefa em
  laço (o `bench`) espera os slots saírem (`wait_exited`) antes de criar de novo.
- **Afinidade:** `Spawn::affinity` é uma máscara de CPUs (zero é qualquer uma), fixa até a tarefa
  sair ([DEBT-0052](../tech-debt/0052-afinidade-so-no-spawn.md)).

### Custo por tarefa e por CPU

| Item | Bytes |
| --- | --- |
| `Slot` de `TASKS` (`.bss`, 32 slots) | cerca de 140 por slot (os atômicos e o `IrqLock` dos metadados) |
| Política por CPU (`Composite<4, 32>`, no heap, criada quando a CPU entra no escalonador) | 3 680 por CPU, a de 4 vagas de FIFO e 32 de EEVDF; 96 por tarefa justa ([SDD 14](14_escalonador.md)) |
| `PerCpu` (`.bss`, 8 blocos, cada um alinhado a 64 bytes) | cerca de 500 por CPU, com a fila (o `Option<Box>` da política), o cache de 32 frames (256) e os contadores |
| `Context` | 8 (x86_64: só o `rsp`) e 104 (aarch64: x19–x30 e `sp`) |
| Pilha mapeada de tarefa | 16 384 (4 páginas, eram 8), mais as tabelas de página que faltarem |
| Por núcleo secundário | x86_64: pilha de 16 KiB mais 3 pilhas IST de 16 KiB cada, mais GDT e TSS (cerca de 170 bytes, no heap); aarch64: 16 KiB mais a pilha de falhas de 16 KiB; as guardas não têm frame. Eram 32 KiB de pilha |
| Área de FPU/SIMD, só da tarefa que usa SIMD | 832 no x86_64 da QEMU (`xsaveopt`, x87 SSE AVX) e 528 no aarch64 |

### Pico de pilha

Toda pilha de thread do kernel (`task::STACK_PAGES`) e a do boot nascem **pintadas** com o padrão
`0x5253_5441_434b_2121` (`kernel/src/stack.rs`); o pico é o quanto do topo o padrão deixou de cobrir, varrendo
do fundo até a primeira palavra diferente. É o `stack_not_used()` do Linux (`CONFIG_DEBUG_STACK_USAGE`, chamado
em `do_exit`: `while (!*n) n++` a partir de `end_of_stack`), com padrão não nulo: o do Linux usa zero, que uma pilha
também escreve, e a pilha dele já nasce zerada. Aqui o frame vem do alocador com lixo, então pinta-se.

| Quando | O que custa | Medido (`dev`, 4 CPUs) |
| --- | --- | --- |
| Primeiro `spawn` de um slot, e as pilhas do boot e dos secundários | Pinta a pilha inteira, uma escrita atômica relaxada por palavra | aarch64 HVF: 13 pilhas no boot (a de 64 KiB e 12 de 16 KiB) em 11 a 24 us, cada uma de 16 KiB em cerca de 1 us (máximo 5 us); x86_64 TCG: 0,3 a 3,3 ms no boot inteiro (TCG só compara o RustOS consigo mesmo) |
| `spawn` que reusa o slot | Varre do fundo a uma palavra por linha de cache, refina na última linha limpa e repinta só o que foi usado; o pico da tarefa anterior entra em `TASK_PEAK` | aarch64 HVF: 0,3 us em média (72 reusos no boot e no `bench`; máximo 0,7 a 2,4 us); x86_64 TCG: 3 us. A varredura palavra a palavra, numa pilha de 32 KiB, custava 1,7 us no HVF |
| `ps`, linha `stacks:` | Varre as pilhas dos slots vivos e dos encerrados (o slot guarda a marca até ser reusado) | fora de qualquer caminho quente |

Pintar só sob demanda não é possível: o pico de uma tarefa se mede *depois* que ela rodou, e a pilha precisava estar pintada antes. A
saída barata foi não repintar tudo a cada `spawn`. A varredura de uma palavra por linha de cache pode errar para menos por uma linha (64 bytes) se a palavra
amostrada for um buraco que a tarefa não escreveu; a linha que sobra suja só faz o pico seguinte sair maior.

O pico aparece no `ps` (coluna `stack`, `3.4/16K`: pico e tamanho) e no boot, na linha
`[rustos] stacks: idle peak X KiB of 16, task peak Y KiB of 16, shell peak Z KiB of 64`: o maior pico de cada tipo desde o boot,
incluídas as tarefas já encerradas (`ociosa`: as de todas as CPUs; `shell`: a tarefa `init`, que corre na pilha de 64 KiB do boot).

**Medida** (QEMU, 4 CPUs, boot completo, `bench` e `fault break`; o IRQ do timer e o IPI de reescalonar caem na pilha da tarefa corrente e já
estão no pico; bytes, o máximo de cada tipo; `dev` e `release`):

| Pilha | aarch64 `dev` | aarch64 `release` | x86_64 `dev` | x86_64 `release` |
| --- | --- | --- | --- | --- |
| Ociosa da CPU de boot (já em regime) | 1 136 | 928 | 1 152 | 848 |
| Ociosa de núcleo secundário (inclui o init do núcleo: `ap_main`, GIC ou LAPIC, timer) | 6 688 | 3 904 | 6 736 | 3 792 |
| Tarefa do autoteste e do `bench` (a maior, `waker`; as outras ficam em 0,3 a 1,3 KiB) | 2 512 | 2 304 | 2 352 | 2 192 |
| Pânico dentro de uma tarefa (`fault cpu`: o relatório todo, a profundidade da tarefa inclusa) | 1 328 | 1 024 | 1 272 | 1 064 |
| Falha na pilha de falhas do aarch64 (`fault page`, relatório de falha, 16 KiB) | 2 352 | 1 888 | n/a | n/a |
| Shell (`init`, na pilha de 64 KiB do boot), em KiB: boot, depois `ps`, depois `bench` | 13,7; 17,1; 23,3 | 16,8; 38,2; 42,6 | 13,5; 17,0; 23,1 | 16,6; 38,1; 42,3 |

`release-small` (`opt-level = "z"`, `make smoke`): tarefa 2,2 (aarch64) e 2,1 KiB (x86_64), ociosa da CPU de boot 0,8, núcleo secundário **8,8 KiB** (o maior de todos os perfis), shell 15,5 no boot e 33,8 depois do `bench`.

`dev` (`opt-level = 1`) usa **mais** pilha que `release` nas tarefas e nas ociosas (2,5 contra 2,3 KiB; 6,7 contra 3,9 KiB), e **bem menos** no shell:
no `release` o `ps` já leva a pilha do shell a 38 KiB e o `bench` a 42,6 de 64 ([DEBT-0071](../tech-debt/0071-shell-usa-dois-tercos-da-pilha-no-release.md)). O
`fault page` no x86_64 roda na pilha do shell e não separa o relatório do resto; o `#BP`, o `#PF` e o pânico do x86_64 não usam as pilhas IST (pico zero
nas três de cada CPU, [DEBT-0072](../tech-debt/0072-pilhas-de-excecao-sem-medida-permanente.md)). Não existe falta de página tratada ainda: a única
é a que vira relatório.

**Decisão: 4 páginas (16 KiB) para toda thread do kernel que não é o boot**, tarefas, ociosa de cada CPU e pilha de cada núcleo secundário (eram 8, 4 e 8).
Regra: o tamanho é a menor quantidade de páginas com pelo menos 2x o pior pico de `dev` mais o pior relatório de falha. Isso é 2 x (2,5 + 2,4) = 9,8 KiB, 3
páginas, e a janela de pilhas pede potência de dois no teste de estouro do aarch64 que o desenho da fase 4 prevê (pilha alinhada ao dobro do tamanho), então 4. A margem é 6,4x o pico de tarefa
e 1,8x o pico mais fundo (o init do núcleo secundário em `release-small`, 8,8 KiB; é um caminho fixo, medido nos três perfis). 2 páginas (8 KiB) ficariam em 1,6x da soma pessimista e nem caberiam no secundário: não. O Linux usa 16 KiB nos dois
alvos (`THREAD_SIZE` é `PAGE_SIZE << 2` no x86_64 e `1 << 14` no arm64), e o estouro aqui vira relatório pela guarda sem mapeamento, não corrompe.

Ganho, medido pelo `mem` (`used`, aarch64 `release`, depois do boot e do `bench`): **720,0 para 544,0 KiB**, 176 KiB a menos. Por tarefa, 16 KiB (50% da pilha);
por núcleo secundário, 16 KiB; os 24 slots de tarefa cheios custariam 384 KiB a menos que antes.

**A fase 4 remede.** Uma syscall e uma falta de página de usuário vão rodar na pilha do kernel da thread, e nenhuma das duas existe neste número
([DEBT-0070](../tech-debt/0070-pilha-de-16-kib-sem-syscall-nem-falta-de-usuario.md)). A medida se repete com o mesmo comando: `make screenshot ARCH=… CMDS="ps;bench"` e a linha `stacks:` do
`make smoke`; para o pânico, uma linha de depuração na pilha onde ele corre (não está no código). O 4a decide se 16 KiB seguem ou se a pilha passa a pedir mais.

## O escalonador

Uma **fila por CPU**: `PerCpu.queue`, um `IrqLock<Queue>` com a política (`Composite<4, 32>`), a
ociosa da CPU e o acordar mais próximo de quem dorme nela. A política só guarda os prontos, o corrente
e quem dorme **na fila dela**, por `TaskId`; a tabela `TASKS` guarda o resto. O kernel avisa
cada evento à fila da CPU dona da tarefa, sempre com o relógio `now_ns`:

| Evento | Chamada à política |
| --- | --- |
| `spawn`, e `init` para o boot | `add(id, params, now)` na CPU menos carregada que a afinidade permite (as ociosas não entram) |
| `park`, `sleep_until` | `sleep(id, now)` na fila da CPU que roda, antes de `schedule` |
| `unpark`, acordar do timer | `wake(id, now)` na fila dona; se a CPU escolhida é outra, `detach` ali e `add_with_lag` lá ([abaixo](#migração-e-balanceamento)). Pinada a uma CPU só, fica na dela sem olhar a carga das outras |
| `yield_now` | `yield_current(now)` |
| `exit` | `remove(id)` |
| IRQ do timer | `tick(now)` |
| Fim de toda IRQ, de `unpark` e de `spawn`, e `schedule()` | `needs_resched()` e `pick_next(now)` ([Preempção](#preempção)) |

A política só tem tarefa pronta, corrente ou dormindo na fila dela, então não há varredura de quem
bloqueia ou saiu. `MAX_TASKS` é a capacidade de cada política, que cabe qualquer tarefa do sistema:
uma tarefa pode migrar para qualquer fila.

`switch_from` é a única função que troca: contabiliza o tempo da que sai, pede `pick_next`
(`None` vira a ociosa), marca `on_cpu` da que entra, rearma o timer, publica a carga, solta o lock da
fila, salva e restaura o estado de SIMD (só de tarefa marcada) e chama `switch`. Recebe o lock da fila **já preso**:
`park`, `sleep_until`, `yield_now` e `exit` o pegam para decidir (bloquear, ceder) e o passam adiante sem soltar, uma
tomada só da decisão à escolha (com o atalho do `wake_target`, -2 ns por troca no HVF). `schedule()` e `schedule_at(now)` pegam o lock e chamam `switch_from`
(a ociosa e a preempção do fim da IRQ). Depois do
`switch`, já na pilha de quem entrou, `finish_switch` baixa o `on_cpu` da que saiu desta CPU (com
release): só então outra CPU pode escolhê-la.
O `unpark` com a tarefa ainda não bloqueada deixa um aviso que o próximo `park` consome,
então "acordar antes de dormir" não perde o acordar (é o que o ping-pong do `bench` usa).

## Estado por CPU

`PerCpu` (`kernel/src/percpu.rs`) é um bloco por núcleo em `static CPUS`. O primeiro `usize` dele é o
próprio endereço, e o `arch/` o guarda onde a CPU o acha sozinha pela porta `hal::CpuLocal`: no x86_64,
`IA32_GS_BASE` e `mov reg, gs:[0]` (sem `rdmsr`); no aarch64, `TPIDR_EL1` e `mrs`. `percpu::current()` é
essa leitura, sem `pure` nem `nomem` (a tarefa pode trocar de CPU entre duas chamadas). A CPU de boot instala o dela
na primeira instrução do `kmain`; cada núcleo secundário, na entrada. Com a IRQ ligada, a tarefa pode migrar entre ler `gs:[0]` e ler um campo do bloco: `task::current`, `is_ready`,
`interrupts::in_irq` e `panic::elsewhere` leem por `percpu::pinned`, que mascara em volta; código que já roda mascarado (o `schedule`, as IRQs) usa `current()` direto.

| Campo | Quem escreve | Para quê |
| --- | --- | --- |
| `index`, `hw_id` | quem sobe a CPU | número da CPU e o destino do IPI (LAPIC ID, ou afinidade do `MPIDR`) |
| `in_irq`, `locks_held` | a própria CPU | as regras de lock abaixo (eram globais na primeira onda) |
| `armed_ns` | a própria CPU | o prazo que está no timer de um tiro desta CPU |
| `load` | a CPU dona, sob o lock da fila | tarefas prontas e correndo: o que as outras CPUs leem sem lock para escolher onde pôr e de onde puxar |
| `current`, `prev` | a própria CPU | a tarefa corrente; a que acabou de sair e ainda tem `on_cpu` |
| `kick_pending`, `idle` | quem manda o IPI / a própria CPU | um IPI de reescalonar a caminho; a CPU dorme |
| `queue`, `frames` | sob o próprio lock | a fila e o cache de frames |
| `stats` | a própria CPU (soma sem `lock xadd`: `sync::bump`) | IRQs, trocas, IPIs, tarefas puxadas |

Contador que só uma CPU escreve é lido e gravado, não somado atomicamente: o `lock xadd` custa de 20 ciclos em metal a centenas de ns no TCG.

## Migração e balanceamento

Quem decide a CPU de uma tarefa (`pick_cpu`): a de **menor carga** entre as ativas que a afinidade permite
(`load` lido sem lock: é estimativa, e o pull corrige o que ela errar), empatando a favor da CPU em que a tarefa dormia.
Ao acordar, `wake_target` fica na CPU de onde a tarefa saiu se a troca dela ainda não terminou (`on_cpu`) ou se
essa CPU está ociosa e a afinidade deixa (cache quente); senão usa `pick_cpu`.

| Quando | Quem | O quê |
| --- | --- | --- |
| `spawn` | a CPU que cria | `add` na CPU menos carregada |
| `wake` | a CPU que acorda (tarefa, IRQ ou timer) | fica ou migra pelo `wake_target` |
| CPU ociosa | a própria, na `idle_loop`, antes de `hlt`/`wfi` | `pull`: puxa de quem tem **2 ou mais** tarefas a mais |
| Fatia da CPU sobrecarregada | a própria, na IRQ do timer, no máximo a cada 2 ms | com 2 ou mais prontas, manda um IPI a uma CPU ociosa, que roda o `pull` ao acordar; com menos, ela mesma puxa |

**Regras de lock e de migração** (o que quebra se mexerem aqui):

1. **Nunca duas filas ao mesmo tempo.** Mover uma tarefa é `detach` sob o lock da fila de origem, soltar, e só então
   `add_with_lag` sob o da fila de destino. Se um dia houver motivo para duas, a ordem é pelo índice da CPU, a menor primeiro.
2. **A tarefa em trânsito é de quem a destacou.** Entre o `detach` e o `add_with_lag` ela está `Ready`, em nenhuma política, e o campo `cpu`
   dela já aponta o destino (escrito ainda sob o lock da origem). Quem a procura (`unpark`, o timer) trava a fila de `cpu`, relê `cpu` depois
   de travar (`lock_owner`) e, achando-a `Ready`, só deixa o aviso (`permit`): nunca a perde nem a coloca em duas filas.
3. **`state`, `permit` e a presença na política mudam só sob o lock da fila da CPU em `cpu`.** `park` consome o aviso e marca `Blocked` sob o mesmo lock em que
   `unpark` decide, então o acordar não se perde.
4. **`on_cpu` impede rodar em dois núcleos.** Sobe sob o lock da fila ao escolher a tarefa; desce na CPU que a deixou, **depois** do `switch` (que salvou o contexto), com release.
   Nenhuma CPU puxa (`pull_candidate` recusa `on_cpu`), migra no acordar nem reusa o slot de uma tarefa com `on_cpu` alto: a pilha e o contexto ainda são dela.
5. **O IPI sai com o lock da fila destino preso, e isso não trava:** ele só escreve o ICR ou `ICC_SGI1R_EL1`; quem o recebe espera o lock da fila no próprio handler.
6. **Ordem de lock global:** heap, cache de frames da CPU, frames; fila (por CPU) e metadados da tarefa (por slot) não se seguram juntos, a não ser a fila, depois o metadado ao nascer.

O `pull` escolhe a tarefa pronta de **prazo mais distante** da CPU mais carregada ([SDD 14](14_escalonador.md)) e o lag viaja com ela.

## IPI

`IpiKind::Reschedule` e `IpiKind::Stop`, pela porta `hal::Ipi` (`X86Ipi`: ICR do x2APIC ou do xAPIC; `Aarch64Ipi`: SGI 0 e 1 por `ICC_SGI1R_EL1`).

- **Reescalonar** (`kick`): depois de pôr uma tarefa numa fila **de outra CPU**, se a fila agora pede troca (`needs_resched`) ou se há prazo mais cedo que o que a CPU
  tem armado, o IPI refaz a decisão dela. `kick_pending` evita mais de um a caminho. A CPU destino, no handler, só desmarca o pendente e rearma o timer; a troca é do
  `preempt`, no fim da IRQ, que é onde o acordar de uma CPU ociosa vira rodar a tarefa. Ociosa acordada por IPI roda o `pull` antes de dormir de novo.
- **Parar** (pânico): ver abaixo.

## Pânico em vários núcleos

O primeiro núcleo a entrar em `panic::begin` ganha a posse (`OWNER`, um `compare_exchange`); o segundo só para. O dono manda o IPI de parada a todas as outras CPUs (`send_all_but_self`) e espera
que elas confirmem (`STOPPED`), por um número limitado de voltas: o relógio pode ser o que travou. As que estão com a interrupção mascarada **girando num `IrqLock`** não ouviriam o IPI:
o giro do lock confere `panic::elsewhere()` e para ali mesmo. Depois disso, o dono toma os locks de serial e tela que um núcleo parado segurava (`break_lock`)
e escreve. `console::print`, `flush`, `key_event` e o `Sink` retornam sem escrever em núcleo que não é o dono do pânico: nenhuma CPU escreve na tela depois do pânico de outra.
A falha de CPU (`on_fault`) usa o mesmo caminho. Se a própria dona entra em pânico de novo (a mensagem, ou o `Display` dela, panica), `begin` toma os locks de volta e `print_panic` cai para a serial direta (`early::print`) quando a instalada está presa; do terceiro relatório da dona em diante (pânico ou falta) só sai uma linha fixa; as outras CPUs que panicam juntas param em silêncio, e sai um relatório só ([SDD 04](04_boot.md#pânico-e-exceção-antes-do-console)). `fault cpu` no shell cria uma tarefa presa a outra CPU que entra em pânico, para conferir.

## Troca de contexto

| | x86_64 | aarch64 |
| --- | --- | --- |
| Guardado | rbp, rbx, r12–r15 empilhados na pilha de quem sai; `Context` é o `rsp` | x19–x30 e `sp` no `Context` (104 bytes) |
| Instrução que troca | `mov rsp, [rsi]` e `ret` | `mov sp, x9` e `ret` |
| Thread nova | 7 palavras no topo da pilha: r15, r14, r13 = `on_return`, r12 = arg, rbx = entrada, rbp, endereço de `rustos_task_start`; o `rsp` fica múltiplo de 16 na primeira instrução | x19 = entrada, x20 = arg, x21 = `on_return`, x30 = `rustos_task_start`, `sp` = topo |
| Trampolim | `rustos_task_start`: `mov rdi, r12; call rbx; call r13; ud2` | `mov x0, x20; blr x19; blr x21; brk #1` |
| Retorno da função da tarefa | `task_exit` → `exit()` | idem |

A IRQ chega com `IF`/`DAIF.I` mascarado e a troca nunca o religa: quem sai de `switch`
volta com a máscara que tinha, e a tarefa nova liga a interrupção em `task_entry`.
`rflags`/`DAIF` não fazem parte do contexto.

## Preempção

Quem decide é a política: `wake`, `add` e `tick` levantam o `needs_resched` dela (fim da
fatia, acordar de prazo mais curto, `RUN_TO_PARITY`). A troca acontece no fim da IRQ,
depois do EOI, ainda mascarada, na pilha da tarefa interrompida: `trap_dispatch` (x86_64) e
`dispatch` (aarch64) chamam `Handlers.preempt` (`task::preempt`), que faz `schedule()` se
`needs_resched()` é verdadeiro; o kernel não tem bandeira própria. Quem volta a rodar sai
do `switch` dentro do próprio handler, retorna pelo mesmo caminho e o quadro da IRQ, que
ficou na pilha dela, é desempilhado.

### Fora da IRQ: `unpark` e `spawn`

`wake` e `add` levantam o `needs_resched` também quando a chamada vem de uma **tarefa**, e antes só a próxima IRQ o
consumia (o fim da fatia da corrente, 100 µs depois no HVF). Agora `unpark` e `spawn` conferem o sinal ao fim, com a
interrupção mascarada, e trocam na hora (`preempt_now`): é o `preempt_enable` do Linux com `CONFIG_PREEMPTION`, que
chama `__preempt_schedule` quando o contador cai a zero com `TIF_NEED_RESCHED` de pé. O critério de quem preempta é o do
EEVDF, o mesmo da IRQ: fatia menor que a da corrente (`PREEMPT_SHORT`), ou ser a escolha de `pick_eevdf`; fatia igual
dentro de uma fatia protegida não preempta, e o ping-pong do `bench` segue sem trocar no `unpark`. Consequência: **`spawn` pode
rodar o filho antes de retornar** (o pai volta quando o filho bloqueia ou a fatia acaba); quem cria tarefa não pode supor que
escreve o estado compartilhado do filho depois do `spawn`.

Equivalente do `preemptible()` do Linux (`preempt_count() == 0 && !irqs_disabled()`): **não troca** se o chamador já tinha a
interrupção mascarada, se está dentro de uma IRQ (o `preempt` do fim dela cuida) ou se segura um `IrqLock`
(`locks_held() != 0`, a regra de lock abaixo). Nesses casos o pedido fica na política e vale na próxima IRQ ou no próximo
`schedule`, como antes. O Linux difere só em adiar para o ponto em que o contador zera; aqui a espera é a da IRQ seguinte, e hoje
nenhum chamador do kernel segura `IrqLock` ao acordar (`notify_input` roda em IRQ): [DEBT-0065](../tech-debt/0065-acordar-com-lock-preso-troca-so-na-proxima-irq.md). Quem acorda numa fila **de outra** CPU já
preemptava, pelo IPI.

### x86_64

As IRQs do APIC (vetores `0x20`, `0x21`, `0x24`, `0xFF`) têm IST zero: caem na pilha da
tarefa (SDD 10, tabela de vetores). Só #DF, NMI e #MC têm pilha própria. O quadro de 22
palavras (`TrapFrame`) fica na pilha de cada tarefa, e o EOI do `arch/` sai antes de
`preempt`, senão o vetor do timer ficaria em serviço enquanto a outra tarefa roda.

### aarch64

O desenho de antes não serve para trocar dentro da IRQ. O Limine entrega em EL1t, e o
kernel rodava em `SP_EL0` com as exceções numa só pilha, `SP_EL1`: duas tarefas
preemptadas dentro do handler empilhariam o quadro na mesma pilha, e `switch` trocaria o
`sp` que o handler usa (`SP_EL1`), não o da tarefa. Alternativas:

| Opção | Por que não |
| --- | --- |
| EL1t e quadro copiado para a pilha da tarefa (`SP_EL0`) | O handler precisa de uma pilha própria para copiar, e o retorno vira um `eret` sintético. Mais asm que a outra |
| EL1t, pilha de exceção por tarefa em `SP_EL1` | Troca `SP_EL1` a cada `switch`, que do EL1 só se escreve com a dança de `SPSel`; duas pilhas por tarefa |
| **EL1h, uma pilha por tarefa em `SP_EL1`** | Escolhida. É o que o Linux faz: `kernel_ventry 1, h, 64, irq` empilha na pilha da tarefa (`arch/arm64/kernel/entry.S`), e o estouro de pilha é pego na entrada e leva a uma `overflow_stack`. O quadro não é copiado |

O kernel passa para **EL1h** em `call_on_stack` (`msr spsel, #1`, depois `mov sp`). A IRQ
é tomada no grupo "EL1 com SPx" (vetores 4–7) e empilha o quadro (272 bytes) na pilha
da tarefa. Falha síncrona e SError (`vector_fault`, vetores 4 e 7) fazem `msr spsel, #0`
antes de empilhar: o `SP_EL0`, que o EL1h não usa, aponta a **pilha de falhas** (4
páginas e guarda, a de `exception::install`), como a `overflow_stack` do Linux, mas
sempre e sem testar o `sp`. Estouro de pilha de tarefa vira relatório ("page fault at ...
write, kernel, not mapped"), conferido com uma recursão numa tarefa, em vez de uma
cascata de faltas na própria entrada.
`sp` de uma exceção que volta (`brk`) é o `SP_EL1` intocado: o `eret` restaura o `SPSel`.

## Regra de lock

**Ninguém cede a CPU com `IrqLock` preso.** `yield_now`, `sleep_until`, `park` e `exit`
não podem ser chamados com um `IrqLock` em mãos: a tarefa seguinte pode querer o mesmo
lock com a interrupção mascarada, e a dona nunca mais roda. `IrqLock` conta os presos **por CPU**
(`PerCpu::locks_held`); `schedule_at` confere `locks_held() == 0` antes de pegar o lock da fila e `switch_from`, que só o da fila esteja preso, **em `debug` e em `release`**: lock preso na troca vira pânico (`context switch with N IrqLock held, expected M`, sem endereço), não deadlock mudo (`fault held` com `panic-tests`, [SDD 06](06_testes.md)). O custo é um load e um compare por troca: aarch64 HVF `release`, 5 boots, linha `switch:` 41 a 43 ns antes e 42 a 44 depois (média 42,0 e 42,6, no ruído do contador, que tem resolução de 41,7 ns por tique); as quatro
funções públicas também recusam chamada de dentro de IRQ (`PerCpu::in_irq`). Dentro de
`switch_from` o lock da fila é solto antes de `switch`. `unpark` e `spawn` só trocam com zero presos (acima). O `IrqLock` é um spinlock de verdade
(`SpinLock`: `compare_exchange` com acquire, escrita com release, espera lendo): mascara a IRQ da CPU local e
gira até a outra CPU soltar. `SpinLock` simples não mascara IRQ: só vale onde o handler nunca o pega (o `KEYBOARD`, o único que
restou, só a IRQ de uma CPU usa). `DEVICES` e `STACKS` são `IrqLock`, porque `map_stack` roda de tarefa em qualquer CPU e
`spawn` serializa o mapeamento das pilhas por outro `IrqLock` (`SPACE`). A ordem entre locks está em [Migração](#migração-e-balanceamento).
O lock conta as tentativas que o acharam preso (`SpinLock::contentions`): é a medida de disputa que o `bench` mostra.

## Tempo

| | x86_64 | aarch64 |
| --- | --- | --- |
| Disparo | TSC-deadline (`IA32_TSC_DEADLINE`) se `CPUID.1:ECX[24]`; senão o contador do APIC em modo de um tiro (divisor 16) | `CNTV_CVAL_EL0` absoluto, desde o começo |
| Relógio | TSC calibrado pelo PIT (SDD 10) | `CNTVCT_EL0` |
| Conversão para ns | `hal::TickScale`: `ticks * mult >> shift`, `mult` arredondado para cima (erro < 1 ns); o `cyc2ns` do Linux | igual |
| Relógio do escalonador | `rdtsc` (que já não é ordenado) | `mrs cntvct_el0` **sem `isb`** (`sched_clock_ns`) |
| Sob TCG | one-shot (o `-cpu max` não anuncia TSC-deadline) | `cntv-cval` |

O timer é **por CPU** (o `CNTV` e o Local APIC são de cada núcleo; `armed_ns` é do `PerCpu`) e sempre de um tiro, e **não há tique periódico depois de `task::init`**
(NO_HZ_IDLE e tarefa única sem tique). O disparo pedido é o menor entre o fim da fatia
da corrente (`next_deadline_ns`, só existe se há alguém esperando) e o próximo acordar
de quem dorme. Quem o arma:

- `interrupts::on_timer` (`task::timer_irq`): acorda quem venceu, chama `tick(now)` e **arma o
  pedido ainda com o lock da fila preso** (quem põe tarefa nesta fila lê o prazo armado sob o mesmo lock;
  armar depois de soltá-lo deixaria um waker de outra CPU ver um prazo velho, não mandar o IPI, e a tarefa esperaria até a hora de `PARKED_NS`); sem pedido, o prazo vai a uma hora (o `start` do `CNTV` faz o mesmo, e o
  `CVAL` vencido manteria a IRQ do aarch64 em serviço). No x86_64 sem TSC-deadline a
  contagem do APIC é de 32 bits e satura: a uma hora vira `u32::MAX` contagens, cerca de
  69 s a 62,5 MHz, e a ociosa acorda no máximo a cada ~69 s, uma IRQ que só rearma. O
  divisor 16 não muda (um maior alongaria o prazo e engrossaria a resolução de 16 ns);
- `schedule`, `unpark` e `spawn`: `arm_no_later_than` só **adianta** o disparo, e um
  disparo que sobrou depois de a corrente mudar custa uma IRQ que não faz nada e rearma.
  Assim o ping-pong do `bench`, que só tem uma tarefa pronta por vez, não escreve no
  timer a cada troca. O timer de **outra** CPU o dono dela rearma: quem põe a tarefa lá manda o IPI de reescalonar quando o prazo novo vem antes do armado.

**O relógio do escalonador** (`task::now`, a leitura de `unpark`, `park`, `sleep_until`, `yield_now` e do timer) não é o da
medida. No aarch64 o `isb` antes do `mrs` custa 8 ns isolado e 11 ns por leitura dentro da troca, de duas por troca
(uma em `unpark` e outra em `park`): tirá-lo levou o ping-pong de 71 a 48 ns. Sem ele a leitura pode sair antes das instruções
anteriores (alguns ns, dentro de um tique de 41,7 ns a 24 MHz) e, em tese, dois `mrs` seguidos saírem fora de ordem; a
política só usa diferenças com `saturating_sub`, a leitura do relógio também (`elapsed_ns`: contador atrás da origem lê zero) e compara `now <= last`, então um passo atrás de um tique não quebra nada.
É o que o Linux faz no x86 (`native_sched_clock` usa `rdtsc()` sem ordem, "the scheduler clock tolerates small errors") e
**não** o que faz no arm64: lá `arch_timer_read_cntvct_el0` é `isb; mrs`, salvo com ECV (`CNTVCTSS_EL0`, autossincronizado).
Aqui o ECV não vale a pena: `ID_AA64MMFR0_EL1.ECV` é 0 no HVF e o caminho não se mediria. `now_ns`, `cycles` e o que mede
tempo (`bench`, `sleep_until` do `bench`, semente) seguem com o `isb`. A conversão por multiplicação e deslocamento
(`hal::TickScale`, o `cyc2ns` do Linux; aritmética pura e igual nas duas ISAs, por isso mora no `hal`, que já tem `redact` e `cmdline`) tirou a divisão de 128 bits: 12,6 para 9,8 ns no `now_ns` do aarch64 (HVF) e 43 para 33 ns
no x86_64 (TCG).

O TSC e o contador do APIC são medidos uma vez, na CPU de boot, e valem para todos os núcleos ([DEBT-0057](../tech-debt/0057-tsc-entre-nucleos-sem-verificacao.md)).

Antes de `task::init` o tique de 100 Hz continua, porque o autoteste `timer:` do boot o
conta. A linha `idle:` do boot mede a ociosa sem tique: o `main` dorme 50 ms e o número
de IRQs do timer no intervalo é 2 (a do tique de boot que sobrou armada e a do acordar),
contra 5 com o tique de 100 Hz.

## FPU e SIMD

O kernel segue soft-float: nenhum código dele usa `xmm` nem `v0`. O mecanismo existe
para a fase 4. **Só a tarefa criada com `simd: true` paga**: tem área própria e, na troca,
`save` ao sair e `restore` ao entrar (`schedule`). A de fora não guarda nada; um registrador
que ela não toca fica como estava. O salvamento é eager (sem `CR0.TS` nem `#NM`): o Linux já o tinha como padrão desde a 4.6
e o desligou de vez em outubro de 2016, na janela da 4.9 (commit `ca6938a1`). O LazyFP
(CVE-2018-3665) é de 2018 e só confirmou a escolha ([SDD 09](09_seguranca.md)). A pesquisa
não achou custo publicado do eager.

### x86_64

| Passo | O quê | Fonte |
| --- | --- | --- |
| `CR0` | `TS` e `EM` zerados; `MP` e `NE` ligados | `fpu__init_cpu_generic` (`fpu/init.c`) zera `TS` e `EM`; `CR0_STATE` (`uapi/asm/processor-flags.h`) tem `MP` e `NE` |
| `CR4` | `OSFXSR` (bit 9), `OSXMMEXCPT` (10), `OSXSAVE` (18, se o CPUID anuncia XSAVE); `TSD` (2) zerado | `fpu__init_cpu_generic`, `fpu__init_cpu_xstate`; bits em `uapi/asm/processor-flags.h`; `rdtsc` liberado: pesquisa `fpu-simd.md`, `cpu/common.c` |
| `XCR0` | x87 e SSE sempre; AVX (bit 2) se `CPUID.0DH.0:EAX[2]`; AVX-512 (bits 5–7) só os três juntos. `XSETBV` depois de `OSXSAVE` | `fpu__init_cpu_xstate` ("Must happen after CR4 setup and before xsetbv()"); `XFEATURE_MASK_AVX512` em `fpu/xstate.h` |
| Tamanho | `CPUID.0DH.0:EBX` **depois** de `XSETBV`, uma vez, arredondado a 64 (mínimo 576) | SDM (CPUID `0DH`), via pesquisa; custo do CPUID `0DH` na pesquisa |
| Salvar | `XSAVEOPT64`; `XSAVE64` se `CPUID.0DH.1:EAX[0]` for zero; `FXSAVE64` sem XSAVE. Restaurar: `XRSTOR64` ou `FXRSTOR64`; `EDX:EAX` = `XCR0` | `XSAVEOPT` em `fpu/xstate.h` do Linux |
| Estado inicial | Área zerada, `FCW` = `0x037F`, `MXCSR` = `0x1F80`, `XSTATE_BV` zero: o `XRSTOR` inicia todos os componentes | `fpstate_init_fxstate` (`fpu/core.c`), `MXCSR_DEFAULT` (`fpu/types.h`) |
| AMX | Fora de `XCR0` | Pesquisa: precisa de permissão por processo |

### aarch64

| Passo | O quê | Fonte |
| --- | --- | --- |
| `CPACR_EL1.FPEN` | Bits 21:20 em `0b11`: sem trap de FP/SIMD em EL1 nem em EL0 (o Limine entrega zero) | `cpu_enable_fpsimd` em `arch/arm64/kernel/fpsimd.c`: `CPACR_EL1_FPEN_EL1EN` (bit 20) e `EL0EN` (bit 21); `isb` depois |
| Salvar | V0–V31 (`stp q`), `FPSR`, `FPCR`: 528 bytes | `struct user_fpsimd_state` (`uapi/asm/ptrace.h`): `vregs[32]`, `fpsr`, `fpcr` e 8 de reserva |
| Assembler | `.arch_extension fp` e `simd` dentro de cada bloco: o alvo é soft-float | — |
| SVE, SME | Não ligados (`ZEN`, `SMEN` seguem zero) | [DEBT-0035](../tech-debt/0035-sve-sme-e-amx-sem-estado-por-tarefa.md) |

### Autoteste

`bench::simd_self_test` cria duas tarefas SIMD. Cada uma põe um valor seu em `xmm0` (x86_64)
ou `v0` (aarch64) e o confere em laço por 40 ms. A 0 nunca cede a CPU (só a preempção
do timer a interrompe); a 1 cede a cada 256 voltas. Passa se nenhuma leitura difere
e a primeira foi escolhida ao menos 3 vezes. Com a restauração desligada, ele falha nas duas
ISAs (`make smoke` sai com erro); `[rustos] task self-test: ok` é linha exigida do boot.

## Medidas

`bench` (comando) e as linhas `switch:`, `scale:` e `alloc storm:` do boot, com o mesmo código (`bench.rs`):

- **Troca de contexto na mesma CPU:** duas tarefas presas à CPU de quem mede passam a vez por `unpark`
  e `park`, 20 000 idas e voltas (2 000 no boot); cada ida e volta são duas trocas. Um aquecimento e o melhor de
  três. Com e sem as duas marcadas SIMD. É o análogo do `lat_ctx` do lmbench com pipe, sem
  o tamanho de conjunto de trabalho.
- **Troca entre CPUs (`cross-core`):** a mesma medida com uma tarefa em cada CPU (afinidade). Cada passagem de vez é um acordar remoto, com IPI e a CPU
  destino saindo de `hlt`/`wfi`; 1/20 das voltas, porque custa dezenas de vezes mais.
- **IPI de ida e volta:** uma tarefa presa a uma CPU manda o IPI de reescalonar a outra, cujo handler devolve outro IPI; mede-se o tempo de uma ida e volta, melhor de quatro rodadas.
- **Escala:** 8 tarefas de CPU (10 ms cada no `bench`, 2 ms no boot, em voltas calibradas na hora) presas às 1, 2 e 4 primeiras CPUs: o tempo total e quantas vezes mais rápido que 1 CPU.
- **Contenção da memória:** cada CPU aloca e devolve 4 frames por volta (e depois um objeto de 64 bytes do heap) ao mesmo tempo, com o cache por CPU desligado e ligado; tempo total e tentativas que acharam o lock preso ([SDD 12](12_memoria_fisica_e_heap.md)).
- **Jitter de acordar:** 200 chamadas de `sleep_until` (20 no boot) de 0,5 a 1,5 ms;
  `agora − pedido` depois do retorno, média e máximo, com o resto do sistema parado e com uma
  tarefa ocupando a CPU.
- **Latência de acordar entre tarefas** (`wake_task_latency`): uma tarefa de fatia 100 µs (o dorminhoco) dorme
  em `park`; outra de fatia base na mesma CPU (o acordador) a acorda e gira até ela rodar; o tempo é o do `unpark` até o
  primeiro carimbo do dorminhoco. O aquecimento não conta e vai até 2 acertos seguidos (no máximo 20 rodadas): a tarefa nova gasta os primeiros ciclos com lag negativo e não preempta;
  200 amostras no `bench` e 20 no boot. Mediana, média e máximo, e quantas vezes o dorminhoco já tinha rodado quando
  `unpark` voltou (`direct`).
- **Aquecimento do `bench`:** o comando gira a CPU 300 ms antes da primeira medida (`Effort::warmup_ms`). Sem isso ele chega
  depois de segundos ociosos e a troca de contexto sai ~1,5 a 3x pior que a do boot no HVF: o hospedeiro mantém a vCPU em
  frequência baixa por centenas de ms (50 ms de laço deixou 63 ns; 150 ms, 50; 300 ms, 42 a 48, o valor do boot).
- **Política:** `pick_next` e `tick` do `Composite` com 8 tarefas prontas e um relógio de
  mentira (`policy_ns`), melhor de 4 rodadas: o custo do código da política no alvo, com o
  `i128` do EEVDF.
- **Tique ocioso:** a linha `idle:` (acima).
- **Autoteste de acordar** (`wake self-test:`, dentro do `smp_self_test`): a mesma medida com 20 amostras. Passa se em 90%
  delas o dorminhoco já tinha rodado quando `unpark` voltou (`direct`); sem a troca no fim do `unpark` são 0 de 20. O critério
  é contar, não um tempo: o x86_64 em TCG `dev` com uma CPU tem dois modos por boot, com a troca de contexto em 3 µs ou em 30 µs
  (medido na revisão do PR, com `RUSTOS_SMP=1`: `dev`, 9 de 64 boots neste código e 8 de 64 na `main`; `release`, 10 de 12 neste código
  com ~10 µs e 9 de 12 na `main` com ~13 µs; não é regressão; sem IRQ por troca nem `rdtsc` lento, a causa não foi isolada, e a hipótese
  é a TLB do TCG batendo no layout sorteado pelo KASLR), e um limite em µs reprovava o modo lento. Imprime média, mediana, máximo e `direct`. Uma amostra em
  umas 350 chega a esperar o fim da fatia (350 µs no HVF: lag negativo do dorminhoco, a regra do EEVDF); a amostra que passa de 5 ms
  conta como "não direta" e o teste segue. É linha exigida do
  boot (`make smoke`).
- **Autoteste SMP** (`smp self-test:`): 8 tarefas rodam, cedem e dormem por 40 ms nas CPUs todas. Cada volta sobe uma bandeira por tarefa e a baixa ao fim: duas CPUs
  rodando a mesma tarefa fariam a segunda ver a bandeira subida. Passa se nenhuma tarefa se perdeu e a bandeira nunca foi vista subida; imprime voltas e migrações.
  Repetido 16 vezes (8 em cada ISA, `dev`) sem falha.

`ps` e `bench` mostram uma linha por CPU: tarefas na fila, tempo ocupado, trocas, IPIs mandados e recebidos e tarefas puxadas; `bench` fecha com a contenção dos locks globais. Os números e a aceleração estão no [SDD 07](07_desempenho_e_recursos.md).

## Fontes

| Afirmação | Fonte |
| --- | --- |
| Vetores de EL1h com a pilha da tarefa e `overflow_stack` | `arch/arm64/kernel/entry.S` do Linux (`kernel_ventry`, `__bad_stack`) |
| TSC-deadline: modo no LVT (`APIC_LVT_TIMER_TSCDEADLINE = 2 << 17`), `mfence` entre o LVT e o MSR em xAPIC | `arch/x86/include/asm/apicdef.h` e `arch/x86/kernel/apic/apic.c` (`__setup_APIC_LVTT`) |
| `IA32_TSC_DEADLINE` = `0x6E0` | `arch/x86/include/asm/msr-index.h` |
| `CPUID.1:ECX[24]` (TSC-deadline), `[26]` (XSAVE), `CPUID.0DH.1:EAX[0]` (XSAVEOPT) | `arch/x86/include/asm/cpufeatures.h` |
| O `-cpu max` do TCG não anuncia TSC-deadline | SDD 10, medido na QEMU 11.1.3 |
| O Linux troca a FPU de forma eager desde o 4.6 | Pesquisa `fpu-simd.md`, seção 3 (espelho do commit `ca6938a1`) |
| Que o IPI do arm64 é uma SGI configurada por CPU (`ipi_setup`) e o IPI de parar existe junto com o de reescalonar | `arch/arm64/kernel/smp.c` do Linux (`IPI_RESCHEDULE`, `IPI_CPU_STOP`, `ipi_setup`; lido em 2026-11-17) |
| Escrever `ICC_SGI1R_EL1`: `TargetList` 15:0, `Aff1` 23:16, INTID 27:24, `Aff2` 39:32, `IRM` bit 40, `RS` 47:44, `Aff3` 55:48 | `ICC_SGI1R_*` em `include/linux/irqchip/arm-gic-v3.h` do Linux (lido em 2026-11-17) |
| `on_cpu` e a espera de que a CPU antiga termine a troca antes de rodar a tarefa em outra | `kernel/sched/core.c` do Linux (`p->on_cpu`, `finish_task_switch`): **o desenho é conhecido do Linux, mas o `core.c` não foi relido para este capítulo** |
| O relógio do escalonador no x86 é `rdtsc()` sem ordem, convertido por multiplicação e deslocamento; o do arm64 usa `isb; mrs` sem ECV | `native_sched_clock` em `arch/x86/kernel/tsc.c` e `arch_timer_read_cntvct_el0` em `arch/arm64/include/asm/arch_timer.h` do Linux (lidos em 2026-11-17, `master`) |
| Trocar ao fim de quem acorda, se o contador de preempção é zero e a interrupção está ligada | `preempt_enable` com `CONFIG_PREEMPTION` e `preemptible()` em `include/linux/preempt.h` do Linux (lido em 2026-11-17) |
| O padrão de pintar a pilha e varrer do fundo atrás da primeira palavra diferente, só que com zero; chamado ao encerrar a tarefa | `stack_not_used()` e `check_stack_usage()` em `kernel/exit.c` do Linux, sob `CONFIG_DEBUG_STACK_USAGE` (lidos em 2026-11-18, `master`) |
| 16 KiB de pilha de thread do kernel nos dois alvos | `THREAD_SIZE_ORDER (2 + KASAN_STACK_ORDER)` em `arch/x86/include/asm/page_64_types.h` e `MIN_THREAD_SHIFT (14 + KASAN_THREAD_SHIFT)` em `arch/arm64/include/asm/memory.h` do Linux (lidos em 2026-11-18, `master`, sem KASAN) |
| Spinlock de verdade com `IrqLock` mascarando a IRQ local | `SpinLock` em `lib/sync`, com teste de 4 threads no host |
