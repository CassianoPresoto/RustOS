# SDD 15 — Tarefas, troca de contexto e preempção

> **O que este capítulo responde:** como o kernel cria, escolhe e troca threads, como
> o timer preempte uma tarefa em cada ISA, onde fica o estado de FPU/SIMD e que regra
> de lock a troca exige.

Fase 3 do [roadmap](08_roadmap.md), uma CPU só. A política de escolha é o `Composite`
(FIFO e EEVDF) de `core/sched`, atrás do trait `Scheduler` ([SDD 14](14_escalonador.md)); este
capítulo é a cola do kernel. Tempo e timer estão no [SDD 10](10_interrupcoes.md); pilhas, no
[SDD 13](13_memoria_virtual.md).

## Quem é quem

| Peça | Onde | Papel |
| --- | --- | --- |
| `Context`, `switch` | `arch/<isa>/src/context.rs` | Salva os registradores callee-saved e o `sp`; `Context::new` monta a thread nova (trampolim `rustos_task_start`) |
| `fpu` | `arch/<isa>/src/fpu.rs` | Liga FPU/SIMD, mede a área de estado, `save` e `restore` |
| `xstate` | `arch/x86_64/src/xstate.rs` | Puro, testado no host: `XCR0` a ligar, tamanho da área, estado inicial |
| `hal::Handlers.preempt` | `core/hal/src/interrupt.rs` | Gancho que o `arch/` chama no fim de toda IRQ, depois do EOI |
| `task.rs` | `kernel/src/task.rs` | Tabela de tarefas, `spawn`, `yield_now`, `sleep_until`, `park`/`unpark`, `exit`, a troca e a preempção |
| `interrupts.rs` | `kernel/src/interrupts.rs` | IRQ do timer: o que o escalonador pede (acordar, fim de fatia) e o rearme do disparo |
| `bench.rs` | `kernel/src/bench.rs` | Autoteste de SIMD, latência de troca e jitter de acordar |
| `ps`, `bench` | `core/shell`, porta `Host` (`tasks`, `bench`) | Os dois comandos do shell |

`hal` não ganhou porta de contexto: nenhum crate de `core/` troca de contexto, e quem
a usa é só o `kernel/`, pelo `platform`. `core/sched` só decide.

## A tarefa

Cada slot de `SCHED` (`MAX_TASKS` = 16) guarda: nome, estado, função e argumento, topo
da pilha, prazo de acordar, um aviso de `unpark` adiantado (permit), tempo de CPU em
ns, quantas vezes foi escolhida, se usa SIMD e a área de FPU. Os parâmetros de
escalonamento (`SchedParams`: classe, nice, fatia) vão para a política em `spawn`. O `Context` fica
fora do lock, em `CONTEXTS`: a troca o usa com o lock já solto. `TaskId` é o número do slot.

| Estado | Quando |
| --- | --- |
| `Free`, `Exited` | Slot disponível para `spawn` (a pilha mapeada e a área de FPU ficam com o slot) |
| `New` | Reservado por `spawn`, sem contexto ainda |
| `Ready`, `Running` | Pronta; ou a que ocupa a CPU |
| `Blocked` | Em `park`, até um `unpark` (ou o aviso já estava lá) |
| `Sleeping` | Em `sleep_until`, até o prazo |

- **Slot 0 é a ociosa**, fora da política (como o `idle` do Linux, que não está em
  nenhuma classe): roda quando `pick_next` devolve `None` e faz `hlt`/`wfi` com a
  interrupção ligada (`enable_interrupts_and_wait`).
- **Slot 1 é o boot.** `task::init` adota a execução do `kmain`, com a pilha de 64 KiB
  que já tinha, como tarefa `init`. Ela roda os autotestes e, em `run_shell`, vira a
  tarefa `shell`, classe justa, nice 0 e fatia de 200 µs (interativa: o prazo virtual
  curto a põe na frente de quem só calcula, e `PREEMPT_SHORT` fura a proteção da fatia
  de uma corrente de 700 µs). O shell dorme em `park` quando a fila de entrada está vazia, e a IRQ
  da serial ou do teclado o acorda (`task::notify_input`).
- **Cada tarefa nova tem pilha própria:** 8 páginas (32 KiB) e uma de guarda sem
  mapeamento, na janela de pilhas (`memory::map_stack`). Reusar o slot reusa a pilha:
  a janela é só de avanço. A ociosa tem 4 páginas.

### Custo por tarefa

| Item | Bytes |
| --- | --- |
| `SCHED` (`.data`: 16 slots, `Composite<4, 16>`) | 3 968 no total nas duas ISAs em `release`, cerca de 250 por tarefa (eram 1 960 com o rodízio); a política pesa 96 por tarefa justa ([SDD 14](14_escalonador.md)) |
| `Context` | 8 (x86_64: só o `rsp`) e 104 (aarch64: x19–x30 e `sp`) |
| Pilha mapeada | 32 768 (8 páginas), mais as tabelas de página que faltarem |
| Área de FPU/SIMD, só da tarefa que usa SIMD | 832 no x86_64 da QEMU (`xsaveopt`, x87 SSE AVX) e 528 no aarch64 |

O pico de uso da pilha não foi medido: [DEBT-0037](../tech-debt/0037-pilha-de-tarefa-sem-medida-de-pico.md).

## O escalonador

`Scheduler` (atrás de um `IrqLock`) tem a tabela e um `Composite<4, 16>`: quatro vagas
de FIFO (tempo real, sem uso no kernel ainda) e 16 de EEVDF. A política só guarda os
prontos e o corrente, por `TaskId` (o slot); a tabela guarda o resto. O kernel avisa
cada evento, sempre com o relógio `now_ns`:

| Evento | Chamada à política |
| --- | --- |
| `spawn`, e `init` para o boot | `add(id, params, now)` (a ociosa não entra) |
| `park`, `sleep_until` | `sleep(id, now)` antes de `schedule` |
| `unpark`, acordar do timer | `wake(id, now)` |
| `yield_now` | `yield_current(now)` |
| `exit` | `remove(id)` |
| IRQ do timer | `tick(now)` |
| Fim de toda IRQ, e `schedule()` | `needs_resched()` e `pick_next(now)` |

A política só tem tarefa pronta ou corrente, então não há varredura de quem dorme,
bloqueia ou saiu, que o rodízio exigia. `MAX_TASKS` é a capacidade da tabela e da política, não o tamanho
de um anel.

`schedule()` é a única função que troca: contabiliza o tempo da que sai, pede `pick_next`
(`None` vira a ociosa), rearma o timer, solta o lock, salva e restaura o estado de SIMD (só de tarefa marcada) e chama `switch`.
O `unpark` com a tarefa ainda não bloqueada deixa um aviso que o próximo `park` consome,
então "acordar antes de dormir" não perde o acordar (é o que o ping-pong do `bench` usa).

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
lock com a interrupção mascarada, e a dona nunca mais roda. `IrqLock` conta os presos
(`sync::locks_held`) e `schedule()` faz `debug_assert_eq!(locks_held(), 0)`; as quatro
funções públicas também recusam chamada de dentro de IRQ (`interrupts::in_irq`). Dentro de
`schedule` o lock do escalonador é solto antes de `switch`. `SpinLock` simples não mascara
IRQ: só vale onde o handler nunca o pega (o `KEYBOARD`, o único que restou, só a IRQ usa).
`DEVICES` e `STACKS` viraram `IrqLock`, porque `map_stack` roda de tarefa e `spawn`
serializa o mapeamento das pilhas por outro `IrqLock` (`SPACE`).

## Tempo

| | x86_64 | aarch64 |
| --- | --- | --- |
| Disparo | TSC-deadline (`IA32_TSC_DEADLINE`) se `CPUID.1:ECX[24]`; senão o contador do APIC em modo de um tiro (divisor 16) | `CNTV_CVAL_EL0` absoluto, desde o começo |
| Relógio | TSC calibrado pelo PIT (SDD 10) | `CNTVCT_EL0` |
| Sob TCG | one-shot (o `-cpu max` não anuncia TSC-deadline) | `cntv-cval` |

O timer é sempre de um tiro e **não há tique periódico depois de `task::init`**
(NO_HZ_IDLE e tarefa única sem tique). O disparo pedido é o menor entre o fim da fatia
da corrente (`next_deadline_ns`, só existe se há alguém esperando) e o próximo acordar
de quem dorme. Quem o arma:

- `interrupts::on_timer` (`task::timer_irq`): acorda quem venceu, chama `tick(now)` e rearma
  para o pedido; sem pedido, o prazo vai a uma hora (o `start` do `CNTV` faz o mesmo, e o
  `CVAL` vencido manteria a IRQ do aarch64 em serviço). No x86_64 sem TSC-deadline a
  contagem do APIC é de 32 bits e satura: a uma hora vira `u32::MAX` contagens, cerca de
  69 s a 62,5 MHz, e a ociosa acorda no máximo a cada ~69 s, uma IRQ que só rearma. O
  divisor 16 não muda (um maior alongaria o prazo e engrossaria a resolução de 16 ns);
- `schedule`, `unpark` e `spawn`: `arm_no_later_than` só **adianta** o disparo, e um
  disparo que sobrou depois de a corrente mudar custa uma IRQ que não faz nada e rearma.
  Assim o ping-pong do `bench`, que só tem uma tarefa pronta por vez, não escreve no
  timer a cada troca.

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

`bench` (comando) e a linha `switch:` do boot, com o mesmo código (`bench.rs`):

- **Troca de contexto:** duas tarefas passam a vez por `unpark` e `park`, 20 000 idas e
  voltas (2 000 no boot); cada ida e volta são duas trocas. Um aquecimento e o melhor de
  três. Com e sem as duas marcadas SIMD. É o análogo do `lat_ctx` do lmbench com pipe, sem
  o tamanho de conjunto de trabalho.
- **Jitter de acordar:** 200 chamadas de `sleep_until` (20 no boot) de 0,5 a 1,5 ms;
  `agora − pedido` depois do retorno, média e máximo, com o resto do sistema parado e com uma
  tarefa ocupando a CPU.

- **Política:** `pick_next` e `tick` do `Composite` com 8 tarefas prontas e um relógio de
  mentira (`policy_ns`), melhor de 4 rodadas: o custo do código da política no alvo, com o
  `i128` do EEVDF.
- **Tique ocioso:** a linha `idle:` (acima).

Os números e a aceleração estão no [SDD 07](07_desempenho_e_recursos.md).

## Fontes

| Afirmação | Fonte |
| --- | --- |
| Vetores de EL1h com a pilha da tarefa e `overflow_stack` | `arch/arm64/kernel/entry.S` do Linux (`kernel_ventry`, `__bad_stack`) |
| TSC-deadline: modo no LVT (`APIC_LVT_TIMER_TSCDEADLINE = 2 << 17`), `mfence` entre o LVT e o MSR em xAPIC | `arch/x86/include/asm/apicdef.h` e `arch/x86/kernel/apic/apic.c` (`__setup_APIC_LVTT`) |
| `IA32_TSC_DEADLINE` = `0x6E0` | `arch/x86/include/asm/msr-index.h` |
| `CPUID.1:ECX[24]` (TSC-deadline), `[26]` (XSAVE), `CPUID.0DH.1:EAX[0]` (XSAVEOPT) | `arch/x86/include/asm/cpufeatures.h` |
| O `-cpu max` do TCG não anuncia TSC-deadline | SDD 10, medido na QEMU 11.1.3 |
| O Linux troca a FPU de forma eager desde o 4.6 | Pesquisa `fpu-simd.md`, seção 3 (espelho do commit `ca6938a1`) |
