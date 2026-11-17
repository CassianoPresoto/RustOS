# SDD 14 — Escalonador

> **O que este capítulo responde:** qual é a porta do escalonador, como funcionam
> o `Eevdf`, o `Fifo` e o `Composite`, que fórmulas e fontes sustentam cada
> escolha, e o que o kernel precisa fazer para ligá-lo.

O `core/sched` (`rustos-sched`) é política pura: `forbid(unsafe_code)`, sem
alocação, sem ponto flutuante, sem crate externo e com capacidade fixa (`const N`).
Ele não conhece relógio, timer nem troca de contexto: recebe `now_ns` e devolve
`TaskId`. A troca de contexto é de `arch/` e a cola é do `kernel/`
([SDD 15](15_tarefas_e_preempcao.md)), que usa o `Composite` desde a fase 3. Os números
estão no [SDD 07](07_desempenho_e_recursos.md).

## A porta: `Scheduler`

Uma CPU por instância. O kernel serializa as chamadas (lock que mascara interrupção)
e passa sempre o relógio monotônico em ns. O trait é compatível com objeto
(`&mut dyn Scheduler`): trocar a política não muda o kernel.

| Método | O que faz |
| --- | --- |
| `add(task, params, now)` | Admite a tarefa, pronta. `Err`: `Full`, `Duplicate`, `WrongClass`, `InvalidPriority` |
| `remove(task)` | A tarefa saiu, em qualquer estado, inclusive a corrente |
| `wake(task, now)` | Bloqueada vira pronta; já pronta ou desconhecida é ignorada |
| `sleep(task, now)` | Pronta ou corrente vira bloqueada |
| `pick_next(now)` | Cobra a corrente, escolhe a próxima e a marca corrente. `None`: CPU ociosa |
| `tick(now)` | Cobra a corrente até `now`. `true`: chame `pick_next` |
| `yield_current(now)` | A corrente cede a vez (pede troca; quem troca é `pick_next`) |
| `next_deadline_ns(now)` | Quando armar o timer de um tiro. `None`: nada o exige |
| `needs_resched()`, `current()` | Estado: algum evento pede troca; quem é a corrente |

Dois métodos além dos do pedido original: `needs_resched` (o `TIF_NEED_RESCHED` do
Linux: `wake`, `add` e `sleep` não devolvem "preempte", só levantam o sinal, que o
kernel confere na saída da interrupção) e `current`. `pick_next` baixa o sinal.

`SchedParams` escolhe a classe: `Fair { nice, slice_ns }` (nice de -20 a 19, preso;
`slice_ns: None` usa a fatia base) ou `Fifo { priority }` (1 a 99; maior vence).

O laço do kernel, para um timer de um tiro (tickless), como `task.rs` o faz:

```text
na interrupção do timer, ou ao sair de qualquer interrupção que mexeu no escalonador:
    sched.tick(now)                       # cobra a corrente
    se sched.needs_resched(): next = sched.pick_next(now); troca de contexto
    rearma o timer em sched.next_deadline_ns(now)   # None: não rearma
```

Com `next_deadline_ns` o escalonador diz quando a fatia da corrente acaba, e só quando
há alguém esperando: tarefa sozinha não gera interrupção de timer (idle e tarefa única
sem tique, o `NO_HZ` do [roadmap](08_roadmap.md)).

## Peso por nice

`weight_of_nice` usa a tabela `sched_prio_to_weight[40]` do Linux, copiada de
`kernel/sched/core.c` (88761 para nice -20, 1024 para nice 0, 15 para nice 19). Cada
nível custa cerca de 25% de peso, o "10% de CPU por nível" do comentário da fonte. O
tempo virtual é `delta_real * 1024 / peso` (`real_to_virtual`, o `calc_delta_fair`).

## `Eevdf` (classe justa)

### Definições

Tudo em ns virtuais (`u64` que dá a volta; comparações por diferença com sinal).

| Símbolo | Definição | Onde está |
| --- | --- | --- |
| `w_i` | peso da tarefa (tabela acima) | `Entity::weight` |
| `v_i` | `vruntime`: tempo de CPU consumido, dividido por `w_i` | `Entity::vruntime` |
| `V` | tempo virtual do sistema: média ponderada dos `v_i` | `avg_vruntime()` |
| `lag_i` | `w_i * (V - v_i)`; em tempo virtual, `V - v_i` | `lag_ns()` |
| `r_i` | pedido de fatia, em ns reais | `Entity::slice_ns` |
| `d_i` | prazo virtual: `ve_i + r_i / w_i` | `Entity::deadline` |

Os fatos de que o código depende, na ordem em que a fonte os dá:

1. **Conservação:** a soma dos lags é zero, `sum(w_i * (V - v_i)) = 0`, o que resolve
   `V = sum(v_i * w_i) / sum(w_i)` (comentário de `avg_vruntime` em `fair.c`).
2. **Média em O(1):** guarda-se `v0 = zero_vruntime`, `sum_w_vruntime = sum((v_i - v0) *
   w_i)` e `sum_weight = sum(w_i)`, sempre incrementalmente. `V = v0 + sum_w_vruntime /
   sum_weight`. O `v0` é levado de volta a `V` a cada fatia esgotada, entrada e saída
   (`recenter`), para as chaves `v_i - v0` ficarem pequenas. Aqui a soma é `i128`:
   o Linux usa `s64` com um `sum_shift` de emergência (`PARANOID_AVG`), e o `i128`
   dispensa o estouro por construção. O `recenter` divide em `i64` quando cabe (mediu
   -26% no `tick` que esgota a fatia).
3. **Elegível** quando `lag >= 0`, ou `v_i <= V`. Sem dividir, para não perder a
   precisão do piso: `sum_w_vruntime >= (v_i - v0) * sum_weight` (`vruntime_eligible`).
4. **Prazo:** `d_i = v_i + r_i / w_i`, renovado quando `v_i >= d_i` (`update_deadline`,
   a fatia acabou e `tick` devolve `true` se há alguém esperando).
5. **Escolha:** o elegível de menor prazo. A corrente entra na disputa (se elegível) e
   só vence se o prazo for *estritamente* menor (`pick_eevdf`).
6. **`RUN_TO_PARITY`:** a corrente é escolhida de novo sem comparar enquanto
   `v < vprot` e ainda elegível, com `vprot = d` fixado em cada `pick_next`
   (`set_protect_slice`). Termina no fim da fatia ou no ponto de lag zero, o que vier
   antes (comentário de `RUN_TO_PARITY` em `features.h`).
7. **`PLACE_LAG` ao acordar** (`place_entity`): a tarefa volta com o lag que saiu,
   mas o lag precisa ser inflado, porque a própria tarefa entra na média e a puxa:

   ```text
   V' = V - w_i*vl / (W + w_i)      vl_efetivo = vl - w_i*vl / (W + w_i)
   para o lag efetivo ser vl:       vl_usado = vl + w_i*vl / W      (a conta da fonte)
   v_i = V - vl_usado               d_i = v_i + r_i/w_i
   ```

   Tarefa nova entra com lag zero e meia fatia (`PLACE_DEADLINE_INITIAL`:
   `d_i = v_i + r_i/(2*w_i)`).
8. **Lag guardado** (`entity_lag`): preso a `±` duas vezes a maior fatia já vista,
   em ns virtuais, para a média não se perder em entradas e saídas.
9. **`yield`** (`yield_task_fair`): a corrente, se elegível e com companhia, faz
   `v = d` e renova o prazo.
10. **Preempção ao acordar** (`wakeup_preempt_fair`): a tarefa que acorda elegível
    preempta a corrente se a fatia dela for menor que a da corrente (`PREEMPT_SHORT`,
    cancela a proteção), ou se, sem proteção, ela for a escolha de `pick_eevdf`. Fatia
    igual dentro de uma fatia protegida não preempta.

Fatia base: **700 µs**, o `sysctl_sched_base_slice` (0,70 ms x `1 + ilog(ncpus)` no
Linux; aqui é uma constante configurável por `EevdfConfig`, sem o fator). Pedido por
tarefa (`fair_with_slice`) preso a 0,1 ms a 100 ms, os limites que o `fair.c` aplica a
`sched_runtime`.

### Estrutura

```text
Eevdf<N>:  IdTable<N> (TaskId -> slot, 8 B)    Entity[N] (56 B)    DeadlineTree<N> (32 B/nó)
```

- **`Entity`** guarda `vruntime`, `deadline`, `vprot`, `slice_ns`, `vlag`,
  `sleep_vtime`, `weight` e o estado (`Sleeping`, `Ready`, `Running`).
- **`DeadlineTree`** é um treap (árvore e heap ao mesmo tempo) ordenado por
  `(prazo, slot)`, com o menor `vruntime` de cada subárvore, como a rb-tree
  aumentada do Linux. A busca desce uma vez: se a subárvore esquerda tem elegível
  (seu mínimo é elegível), vai para ela; senão, se o nó é elegível, é ele; senão,
  direita. Vale porque "elegível" é monótono em `v`. Só as tarefas prontas estão na
  árvore; a corrente fica fora dela, mas a soma a inclui, como em `avg_vruntime`.
- **`IdTable`** acha o slot de um `TaskId` tentando `id % N` primeiro: com ids densos
  é O(1); o pior caso é uma varredura de `N`.
- **Prioridade do treap:** xorshift32 com semente fixa dentro da árvore, sem fonte de
  aleatoriedade. Determinístico, e a profundidade esperada é ~2 ln n.

Custo por tarefa: 56 + 32 + 8 = **96 bytes** (`size_of::<Eevdf<512>>()` = 49 232).
Medidas de tempo em [SDD 07](07_desempenho_e_recursos.md).

### Escolha: árvore, medida

Decisão tomada com número: a varredura linear de `N` slots custa 11,8 ns com 8
tarefas, 90 ns com 64 e 815 ns com 512; a descida do treap, 2,3, 2,8 e 5,8 ns
(mesma máquina e comando, SDD 07). O treap paga a atualização (inserção e remoção a
cada troca), que domina o `pick_next` (41, 108 e 136 ns): continua bem abaixo da
varredura a partir de 64 tarefas e é o que o 512 pede.

### O que o `Eevdf` faz diferente do Linux, de propósito

| Linux (`fair.c`) | Aqui | Por quê |
| --- | --- | --- |
| rb-tree aumentada com `min_vruntime`, `min_slice`, `max_slice` | treap com `min_vruntime` | a busca não usa os outros dois; a fatia mínima da fila só limita a proteção |
| `DELAY_DEQUEUE` + `DELAY_ZERO`: quem dorme sem elegibilidade fica na fila e queima o lag negativo | `decay_lag` no `wake`: o lag negativo soma o tempo virtual dormido, até zero; lag positivo é preservado | mesmo efeito observável ao acordar, sem manter dorminhoco na soma e na árvore (O(1)); sem isso o lote que dorme 5 ms continuava com o lag da última fatia |
| `vprot` limitado à menor fatia da fila (`RUN_TO_PARITY` recente) | `vprot = prazo` da corrente | é o que o pedido descreve e o que a doc do `features.h` diz |
| limite do lag: `max_slice` da fila mais `TICK_NSEC` | duas vezes a maior fatia já vista | não há `TICK_NSEC` (o kernel é tickless) |
| soma `s64` e `sum_shift` | soma `i128` | sem estouro por construção; custo de código medido no SDD 07 |
| grupos (`cgroup`), `latency-nice`, `NEXT_BUDDY`, `PICK_BUDDY` | não existem | fase 9 e fora do escopo |

`latency-nice` não entrou no Linux: a `sched_attr` do master lido não tem esse
campo. O que entrou é o pedido de fatia (`sched_runtime`), e é isso que o RustOS usa
para a tarefa interativa pedir fatia curta.

## `Fifo` (tempo real)

SCHED_FIFO de `sched(7)`: prioridades 1 a 99 (maior vence), sem fatia.

- A corrente roda até bloquear, ceder ou ser preemptada por prioridade **maior**.
- Preemptada, volta à **cabeça** da lista da sua prioridade e retoma quando as maiores
  bloquearem. Ao acordar, entra no **fim** da lista.
- `yield_current` só cede para a mesma prioridade; nunca desce.
- Estrutura: lista duplamente encadeada por prioridade (`heads`, `tails` e os `prev`/
  `next` dentro da tarefa) mais um `u128` de listas não vazias. Escolher a maior
  prioridade é uma instrução (`ilog2`). Tudo O(1); 6 bytes por tarefa e 8 da tabela
  (`size_of::<Fifo<512>>()` = 7 600, com as 99 listas).
- `next_deadline_ns` é `None`: FIFO nunca expira por tempo.

## `Composite`: o tempo real antes da justa

`Composite<RT, FAIR>` guarda um `Fifo<RT>` e um `Eevdf<FAIR>` e escolhe pelo
trait; só uma classe tem corrente por vez (a outra recebe `put_prev`, que devolve a
corrente à disputa e cobra o tempo).

- **Preempção:** tarefa FIFO que acorda ou é admitida preempta a justa na hora.
- **Teto:** o tempo real pode usar `runtime_ns` por janela de `period_ns`. O padrão
  (`RtBudget::LINUX`) é 950 ms por 1 s: **95%**, os valores de `sched_rt_runtime_us` e
  `sched_rt_period_us` em `sched(7)` e em `sched-rt-group`. Estourado o teto, o
  `Composite` tira o tempo real (que volta à cabeça da lista) e roda a justa até
  a janela virar. Com `runtime >= period` não há teto (o `-1` do Linux).
- **Só vale com justa esperando:** se nenhuma tarefa justa está pronta, o tempo real
  continua. Proteger a justa só importa quando ela quer a CPU, e parar a CPU não
  protege ninguém. É a leitura do Linux atual: o `sched_init_dl_servers` do `deadline.c`
  reserva 50 ms por 1000 ms (`fair_server`) para a classe justa quando ela tem
  trabalho, e o `rt.c` do master inicializa `sysctl_sched_rt_runtime` em 1 s por 1 s; a
  doc e a `sched(7)` seguem dizendo 950 ms, e o RustOS segue a doc.
- **Contabilidade:** o tempo do tempo real é cobrado a cada chamada com `now_ns`, e
  a janela vira em múltiplos de `period_ns` desde 0. `next_deadline_ns` devolve o
  instante em que o teto estoura (e o fim da janela, para o tempo real esperando),
  então o estouro é exato com timer de um tiro e, com tique de 1 ms, passa no máximo
  um tique.

## `RoundRobin`

A referência e o fallback (o kernel o usou até ligar o `Composite`): fila circular (`RunQueue`) e fatia fixa de 4 ms
(`with_slice` muda). Ignora a classe e o nice. `sleep` e `remove` custam O(n) (tiram
do meio da fila).

## Testes

Todos no host (`make test`), em `core/sched/src`:

| O que | Onde | O que garante |
| --- | --- | --- |
| Bordas | cada módulo | tarefa única, todas dormindo, capacidade cheia, `TaskId` desconhecido, duplicado, remover a corrente, prioridade inválida, classe errada |
| Treap contra varredura | `tree.rs` | 20 000 inserções e remoções aleatórias, com a volta do `u64` |
| Justiça | `scenarios.rs` | 2 a 8 tarefas com nices de -20 a 19 por 4 s simulados: a fatia de CPU segue o peso (0,1% a 0,3% de tolerância) |
| Latência | `scenarios.rs` | com 4 tarefas sempre prontas e timer de um tiro, a que dorme e pede fatia de 0,1 ms espera 2,5 µs em média contra 863 µs da de lote (fatia de 6 ms); com fatias iguais e tique de 1 ms, a mesma tarefa espera 0,8 ms no EEVDF contra 15,8 ms no rodízio |
| Tempo real | `scenarios.rs` | FIFO preempta a justa; teto de 95% com justa esperando (0,9500/0,0500 em 10 s); sem justa, 100% |
| Propriedade | `property.rs` | 300 sementes de 400 eventos aleatórios (add, remove, wake, sleep, tick, yield, pick) contra um modelo: nunca escolhe quem dorme; a árvore bate com a varredura; as somas incrementais batem com a recontagem; a soma dos lags fica em `(-W, 0]`; sempre há um elegível |
| Benchmark | `bench.rs` (`#[ignore]`) | ns por `tick`, `pick_next` e `sleep`+`wake` com 8, 64 e 512 tarefas; árvore contra varredura |

A simulação (`sim.rs`) é o laço do kernel contra tarefas que só gastam tempo, com
tique periódico ou timer de um tiro. Mutar o código (peso da soma, cobrança da
corrente) faz a propriedade falhar, e foi como se conferiu que o teste pega.

## Fontes

- [docs.kernel.org, EEVDF Scheduler](https://docs.kernel.org/scheduler/sched-eevdf.html):
  lag, elegibilidade, prazo virtual, tarefas de fatia curta, decaimento do lag de quem
  dorme (lido em 2026-11-17).
- `kernel/sched/fair.c` (master, lido em 2026-11-17): `avg_vruntime`,
  `vruntime_eligible`, `entity_eligible`, `entity_lag`, `place_entity`, `pick_eevdf`,
  `update_deadline`, `set_protect_slice`, `yield_task_fair`, `wakeup_preempt_fair`,
  `sysctl_sched_base_slice = 700000`.
- `kernel/sched/features.h`: `PLACE_LAG`, `PLACE_DEADLINE_INITIAL`, `RUN_TO_PARITY`,
  `PREEMPT_SHORT`, `DELAY_DEQUEUE`, `DELAY_ZERO`.
- `kernel/sched/core.c`: `sched_prio_to_weight`.
- [sched(7)](https://man7.org/linux/man-pages/man7/sched.7.html) (SCHED_FIFO, as regras
  de posição na lista, 950 000 de 1 000 000) e
  [sched-rt-group](https://docs.kernel.org/scheduler/sched-rt-group.html).
- `kernel/sched/rt.c` e `kernel/sched/deadline.c` (master): o teto e o `fair_server`.
- [LWN, An EEVDF CPU scheduler for Linux](https://lwn.net/Articles/925371/), secundária.

## O que o kernel precisa saber

- **`i128`:** a soma e a comparação de elegibilidade usam `i128`; a divisão em 128
  bits vem de `compiler_builtins` (`__divti3`), que o alvo bare-metal já traz. Conferido
  com o kernel ligado: `__divti3` e `__udivti3` linkam no x86_64 e no aarch64, em `release`
  e `release-small`, e `make smoke ARCH=all` passa. Em TCG a divisão é emulada e pesa
  ([DEBT-0041](../tech-debt/0041-escalonador-do-x86-sem-medida-fora-do-tcg.md)).
- **Uma CPU por instância**, sem lock interno ([DEBT-0039](../tech-debt/0039-escalonador-de-uma-cpu.md)).
- **Medida no alvo:** `pick_next` e `tick` no `bench` do kernel (34 a 35 e 2 ns no aarch64
  HVF, com 8 tarefas prontas), a latência de acordar e a troca de contexto, no
  [SDD 07](07_desempenho_e_recursos.md). O custo da política na troca de contexto:
  [DEBT-0042](../tech-debt/0042-troca-de-contexto-lenta-com-eevdf-e-relogio.md).
- **Como o kernel o usa:** `add` em `spawn`, `sleep` em `park` e `sleep_until`, `wake` em
  `unpark` e no acordar do timer, `remove` em `exit`, `tick` na IRQ do timer, `needs_resched`
  no fim de toda IRQ e `next_deadline_ns` junto com o próximo acordar no disparo do timer.
  A ociosa fica fora da política; o shell pede fatia de 200 µs.
- **Cobrança pelo relógio:** a precisão do `vruntime` é a do `now_ns` que o kernel
  passa. Quantum menor que 1 ns virtual por peso arredonda para baixo; com o peso
  máximo (88 761) isso começa abaixo de ~87 ns reais por chamada.
