# SDD 17 — Processos, isolamento e syscalls (desenho da fase 4, aguardando decisão)

> **O que este capítulo responde:** como o kernel passa a rodar código em anel 3 e
> EL0 com espaço de endereçamento próprio, como o programa chama o kernel (entrada
> de syscall, handles, ABI nativa), como o primeiro programa chega à memória, e o
> que o dono precisa decidir antes de qualquer código.

**Estado: Draft. Nada aqui está implementado.** Este capítulo nasce num PR em
Draft só com docs porque a ABI de syscall e o formato de executável são contrato
que o mundo de fora enxerga (caso 6 do [`AGENTS.md`](../../AGENTS.md)), e crate
novo em `core/` e pasta nova de primeiro nível são o caso 1. O merge do Draft é a
aprovação; as [decisões pedidas ao dono](18_fase_4_decisoes_pedidas.md) estão no capítulo seguinte. O
código vem em PRs seguintes, e este capítulo passa a descritivo à medida que cada
um entra.

Fase 4 do [roadmap](08_roadmap.md). Pesquisa de origem, com as marcas de
verificação: `syscalls-processes.md` (2026-11-17). O modelo de controle de acesso
já está decidido: **modelo A** ([SDD 09](09_seguranca.md)), handles com direitos
que só diminuem, sem uid/gid no núcleo.

## Como ler as marcas

| Marca | Significa |
| --- | --- |
| **Verificado** | Lido numa fonte primária (spec, código do Linux, doc do projeto) nesta pesquisa; a fonte vem junto |
| **Verificado (secundária)** | Lido num espelho ou resumo de terceiro; confirmar antes de codar |
| **NÃO VERIFICADO** | Não confirmado em fonte; **não** é premissa do desenho. Dito o que ler antes de codar |
| **Proposta** | Desenho do RustOS, sem fonte a citar. Vale o que a decisão do dono disser |
| **Já é assim** | Fato do código de hoje (caminho dado) |

## Estado e escopo

### O que o código de hoje já dá à fase

| Fato | Onde | Consequência |
| --- | --- | --- |
| A GDT já tem dados de usuário (`0x18`) antes do código de usuário (`0x20`), "a ordem que o `sysret` espera" | `arch/x86_64/src/segments.rs` (**Já é assim**) | `IA32_STAR[63:48] = 0x10` serve; ver [x86_64](#x86_64-syscallsysret) |
| SMEP, SMAP e UMIP ligados; PAN no aarch64 | `platform::harden` ([SDD 13](13_memoria_virtual.md)) | O kernel já não executa nem toca página de usuário por engano |
| Metade baixa vazia: x86_64 com as 256 entradas de baixo sem uso; aarch64 com `TCR_EL1.EPD0` desligando o `TTBR0_EL1` | [SDD 13](13_memoria_virtual.md) | O espaço do usuário nasce nessa metade; no aarch64 o `EPD0` muda |
| UXN em toda folha do kernel no aarch64 | [SDD 13](13_memoria_virtual.md) | Página do kernel nunca executa em EL0 |
| `PageFlags` não tem bit de usuário; o mapeador recusa W+X | `core/hal/src/paging.rs` | `PageFlags` ganha `user`; o W^X de usuário vem de graça |
| `Fault` não diz de que anel veio | `core/hal/src/interrupt.rs` | Ganha `from_user` ([falhas de usuário](#falhas-de-usuário)) |
| `Context`/`switch` de thread do kernel, `simd` por tarefa, EEVDF | [SDD 15](15_tarefas_e_preempcao.md) | O processo de usuário é uma tarefa com mais estado |
| `MAX_TASKS = 16` e `Composite<4, 16>` | `kernel/src/task.rs` ([SDD 15](15_tarefas_e_preempcao.md)) | Pouco para processos; a fase sobe a capacidade (ver [o que muda no kernel](#o-que-muda-no-kernel)) |
| Pilha de falha do aarch64 em `SP_EL0` (o `vector_fault` faz `msr spsel, #0` e empilha lá); EL1h com `SP_EL1` por tarefa | [SDD 15](15_tarefas_e_preempcao.md) | Em EL0, `SP_EL0` é a pilha do **usuário**: o `vector_fault` passa a pegar a pilha de falhas pela área por CPU ([aarch64](#aarch64-svc)) |
| `#DF`, NMI e `#MC` têm IST (IST1 a IST3); `#DB` não | `arch/x86_64/src/traps.rs` | `#DB` ganha IST própria e entrada paranoica ([exceções do anel 3](#exceções-vindas-do-anel-3)) |
| Janela de pilhas só de avanço | [SDD 13](13_memoria_virtual.md) | Criar e matar processos em laço esgota endereço; precisa reciclar |

### O que a fase entrega, por fatia (proposta)

A fase é grande demais para um PR. Cada fatia é um PR com `make pr` verde e um
número novo no [SDD 07](07_desempenho_e_recursos.md).

| Fatia | Entrega | Pronto quando |
| --- | --- | --- |
| 4a | Entrada de syscall e saída para anel 3/EL0 nas duas ISAs; `PageFlags.user`; `Fault.from_user`; `#DB` com IST; cópia usuário↔kernel; espaço de **um** processo, sem objetos; `nop`, `clock_get_monotonic`, `random_get`, `process_exit` e **`debug_write(ptr, len)` provisória, sem handle**; um programa embutido no kernel só para esta fatia | `hello` imprime de anel 3 e de EL0; falha de usuário mata o processo e o kernel segue (**paga** [DEBT-0012](../tech-debt/0012-excecao-de-usuario-so-para-a-cpu.md)) |
| 4b | `core/cpio`, `core/elf`; a árvore de regiões em `core/mm/src/vmar.rs`, **interna ao kernel** (o carregador mapeia frames direto, sem objeto VMO nem VMAR e sem handle); módulo do Limine; `user/` (runtime e `init`); o xtask monta o initramfs | O `init` do initramfs roda, imprime pelo `debug_write` provisório e sai; o smoke exige a linha dele |
| 4c | `core/object`; handles e direitos; **VMO e VMAR como objetos**; `debug_write` ganha o argumento `console` (muda a assinatura da 4a: a ABI é provisória até aqui); página zero; ASLR; `process_create`, `thread_create`, `process_kill`, `object_wait_one`, canais; o `init` recebe os handles pelo canal de boot | Um processo cria outro, que escreve no console por um handle que recebeu |
| 4d | SIMD de usuário, canal de eventos do processo, teste de `xmm0`/`v0` entre processos | Dois processos preservam `xmm0`/`v0`; o culpado de uma falha chega ao pai (**paga** [DEBT-0043](../tech-debt/0043-simd-sem-trap-de-uso-indevido.md)) |
| 4e | KPTI, PCID/ASID, mitigações de CPU ([seção](#mitigações-de-cpu)), relatório no boot | Linha `mitigations:` no boot; custo de cada uma medida |
| 4f | As medidas da fase ([abaixo](#medidas-da-fase)) e este capítulo virando descritivo | Fecha a fase: "syscall nula medida; processo que toca o kernel morre sem derrubar nada" |

**A ABI da 4a e da 4b é provisória.** Só `process_exit`, `nop`, `clock_get_monotonic` e
`random_get` já têm a forma final; `debug_write` muda na 4c. A ABI nativa é experimental
até o fim da fase 5, então a mudança não quebra contrato.

### Divergências em relação ao roadmap atual (propostas)

O [SDD 08](08_roadmap.md) descreve a fase 4 antes deste desenho. Três pontos mudam
se o dono aprovar; o roadmap só muda no PR que os torna verdade.

| O roadmap diz | O desenho propõe | Por quê |
| --- | --- | --- |
| "VFS mínimo, initramfs e tmpfs" | Sem VFS na fase 4: o initramfs é um arquivo lido por quem o recebe como VMO; VFS e tmpfs entram na fase 5, com a tabela de descritores | O modelo nativo não tem caminho: tem handle. VFS existe para a ABI Linux |
| "sinais mínimos" | Sem sinais POSIX na ABI nativa; falha vira mensagem num canal ([decisão 6](18_fase_4_decisoes_pedidas.md#decisão-6--falha-de-usuário-canal-de-eventos-ou-sinal-síncrono)) | Sinal exige o sigframe, e o sigframe exige decidir antes o pedido de AMX e o VL do SVE ([SDD 08](08_roadmap.md#decisões-futuras-casos-do-agentsmd)); sem sinal, as duas decisões esperam a fase 5 |
| "COW no fork" | COW só a partir da página zero compartilhada; `fork` e o COW inteiro vêm com a ABI Linux (fase 5) | A ABI nativa não tem `fork`: cria processo vazio e o preenche |

## Entrada de syscall por ISA

### x86_64: SYSCALL/SYSRET

**Verificado** (Intel, espelho [felixcloutier](https://www.felixcloutier.com/x86/syscall) e [sysret](https://www.felixcloutier.com/x86/sysret); secundária):

- `SYSCALL`: `RCX := RIP`, `R11 := RFLAGS`, `RFLAGS &= ~IA32_FMASK`, `RIP := IA32_LSTAR`,
  CS e SS de `IA32_STAR[47:32]`. **O `RSP` não muda**: o software troca a pilha. `#UD`
  se `EFER.SCE = 0`.
- `SYSRET` (64 bits): `RIP := RCX`, com `#GP(0)` se `RCX` não for canônico; `RFLAGS := (R11 & 3C7FD7H) | 2`;
  `CS := (STAR[63:48] + 16) OR 3` e `SS := (STAR[63:48] + 8) OR 3` ("RPL forced to 3");
  `CPL := 3`; **não muda o `RSP`**.

**Verificado** (Linux: `entry_64.S`, `cpu/common.c`, `syscall_64.c` e `msr-index.h`):

| Item | O que o Linux faz | Onde |
| --- | --- | --- |
| Convenção | `rax` = número; `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9` = argumentos; `rcx` = retorno e `r11` = `rflags` (o hardware sobrescreve os dois) | comentário de `entry_SYSCALL_64` |
| Ordem de entrada | `swapgs`; guarda o `rsp` do usuário num rascunho por CPU (`tss.sp2`); troca o CR3 (KPTI); carrega o `rsp` do topo da pilha da tarefa (`cpu_current_top_of_stack`); monta o quadro com `SS`, `RSP`, `R11`, `CS`, `RCX` | `entry_SYSCALL_64` |
| `FMASK` | Limpa CF PF AF ZF SF TF **IF** DF OF IOPL NT RF **AC** ID: "clear as much as possible to minimize user space-kernel interference". Por isso o `CLAC` não é preciso na entrada | `idt_syscall_init` |
| `STAR` | `(__USER32_CS << 16) \| __KERNEL_CS` | `syscall_init` |
| MSRs | `EFER` `0xC0000080` (`SCE` = bit 0), `STAR` `0xC0000081`, `LSTAR` `0xC0000082`, `SYSCALL_MASK` `0xC0000084`, `FS_BASE` `0xC0000100`, `GS_BASE` `0xC0000101`, `KERNEL_GS_BASE` `0xC0000102`, `IA32_ARCH_CAPABILITIES` `0x10A` com `RDCL_NO` no bit 0 ("not susceptible to Meltdown") | `msr-index.h` |
| Saída por `SYSRET` | **Só se** `regs->cx == regs->ip` e `regs->r11 == regs->flags` ("SYSRET requires RCX == RIP and R11 == EFLAGS"); `cs == __USER_CS` e `ss == __USER_DS`; `regs->ip < TASK_SIZE_MAX` ("SYSRET with non-canonical RCX/RIP will #GP in kernel space" em Intel); e `flags` sem `RF` e sem `TF` (o `SYSRET` não restaura `RF`, e restaurar `TF` trapeia logo depois). Senão `IRET`. A função devolve `true` para `SYSRET` | `do_syscall_64`, `syscall_64.c` |
| Por que `IRET` na dúvida | "When user can change pt_regs->foo always force IRET. That is because it deals with uncanonical addresses better. SYSRET has trouble with them due to bugs in both AMD and Intel CPUs" | comentário de `entry_SYSCALL_64` |
| Saída com KPTI | Volta pela **pilha do trampolim** (`TSS_sp0`), troca para o CR3 do usuário e só então `swapgs; sysretq` | `syscall_return_via_sysret` |
| PCID | `CR4.PCIDE` ligado se a CPU anuncia `PCID` | `cr4_init` |

**CVE-2012-0217** (Xen XSA-7, **verificado**: [advisory](https://xenbits.xen.org/xsa/advisory-7.html)): `SYSRET`
para `RIP` não canônico, em CPU Intel, gera a exceção num estado indevido (com o
`RSP` do usuário já carregado e ainda em anel 0) e dá escalada de privilégio; a
advisory diz que CPU AMD não é afetada. A doc do `SYSRET` dá as saídas: checar o
`RCX` antes, impedir que `SYSCALL` salve um `RCX` não canônico, ou tratar o `#GP`
numa pilha à parte.

**Desenho (proposta):**

| Passo | O quê |
| --- | --- |
| `EFER.SCE` | Ligado no boot, em cada CPU |
| `STAR` | `(0x10 << 48) \| (0x08 << 32)`: com a GDT de hoje, `SYSRET` dá `SS = 0x18 \| 3` e `CS = 0x20 \| 3` (a conta `+8`/`+16` é a **verificada** acima), que são `USER_DATA` e `USER_CODE` de `segments.rs` |
| `LSTAR` | `rustos_syscall_entry`, na seção `.entry.text` ([KPTI-ready](#o-trampolim-e-o-caminho-pronto-para-kpti)) |
| `FMASK` | A máscara do Linux acima. Interrupção só liga quando a pilha da tarefa já é a do kernel |
| Pilha | A entrada guarda o `rsp` do usuário em `gs:[user_rsp]`, carrega `gs:[kernel_rsp]` e monta o **mesmo** `TrapFrame` de 22 palavras que as IRQs já usam ([SDD 15](15_tarefas_e_preempcao.md)), com `SS`, `RSP`, `R11` como `RFLAGS`, `CS`, `RCX` como `RIP` |
| `kernel_rsp` | A troca de contexto escreve o topo da pilha da tarefa que entra em dois lugares: `gs:[kernel_rsp]` e `TSS.rsp[0]` (o hardware carrega `RSP0` ao subir de anel numa exceção ou IRQ). Hoje a TSS não usa `rsp[0]` |
| Despacho | `rax` indexa a tabela de syscalls **depois do clamp de índice** ([mitigações](#mitigações-de-cpu)); número fora da tabela devolve `NotSupported` e o processo **segue** |
| Saída rápida | As seis condições do Linux, acima, com `RCX < USER_TOP` no lugar de `ip < TASK_SIZE_MAX` (todo valor abaixo de `USER_TOP` é canônico; vale em toda CPU, não só Intel). Na fase 4 nada altera o quadro, então `rcx == rip` e `r11 == rflags` valem por construção; a função confere assim mesmo, porque o mecanismo equivalente ao `ptrace` da fase 5 vai poder alterá-lo |
| Saída lenta | Qualquer outro caso: `IRETQ`. Na fatia 4a: **sempre `IRETQ`**; o `SYSRET` entra com o número que o justifique |

A checagem é função pura em `arch/x86_64/src/sysret.rs`, testada no host com os casos de
borda: `USER_TOP - 1`, `USER_TOP`, `0x0000_8000_0000_0000`, `0xFFFF_8000_0000_0000`,
`u64::MAX`, `rcx != rip`, `r11 != rflags`, seletor trocado, `RF` e `TF`.

**`swapgs` e o estado por CPU.** **Verificado** (Linux, [entry_64](https://docs.kernel.org/arch/x86/entry_64.html)): `swapgs`
é "especially tricky": roda uma vez por entrada e não aninha; a checagem barata é o
`CS` salvo na pilha, e a cara é ler `IA32_GS_BASE` em contexto atômico (NMI). Regra do
desenho: **o kernel nunca roda com o `GS` do usuário.**

| Entrada | Como sabe se precisa de `swapgs` |
| --- | --- |
| `SYSCALL` | Sempre vem de anel 3: `swapgs` incondicional |
| Exceção ou IRQ por vetor | `CS` do quadro com `RPL = 3`, antes de tocar em `gs`. **Todo stub de vetor de hoje (`traps.rs`) ganha esse teste** na entrada e na saída |
| NMI, `#DF`, `#MC`, `#DB` (IST) | Podem vir de qualquer estado: **entrada paranoica**. Lê `IA32_GS_BASE` e compara com o endereço da área por CPU (o caminho caro) |

`IA32_KERNEL_GS_BASE` guarda o `GS` do usuário enquanto o kernel roda, e `IA32_GS_BASE`
aponta a **área por CPU**. `CR4.FSGSBASE` fica desligado (o usuário não escreve `FS`/`GS`
base; a fase 4 não tem TLS, ver [carregador](#executável-e-carregador)).

**Ring 3 sem E/S de porta:** `RFLAGS` inicial do usuário é `0x202` (só `IF`; `IOPL = 0`)
e `TSS.iomap_base` precisa ficar além do limite da TSS. Conferir os dois no PR da 4a
(a TSS de hoje já tem `iomap_base`).

#### Exceções vindas do anel 3

**Regra: toda exceção vinda do anel 3 que não é syscall nem IRQ vira `Fault`** com
`from_user = true`; a única que pode resolver e voltar é a falta de página dentro de uma
região do processo. **Verificado** (vetores de `asm/trapnr.h`; `#DB` com IST em
`arch/x86/kernel/idt.c`, `ISTG(X86_TRAP_DB, …, IST_INDEX_DB)`; a macro `idtentry_mce_db`
de `entry_64.S` usa o caminho normal de entrada se vem de usuário e o paranoico se vem do
kernel).

| Vetor | Origem em anel 3 | Origem no kernel |
| --- | --- | --- |
| 0 `#DE`, 3 `#BP`, 4 `#OF`, 5 `#BR`, 6 `#UD`, 7 `#NM`, 10 `#TS`, 11 `#NP`, 12 `#SS`, 13 `#GP`, 16 `#MF`, 17 `#AC`, 19 `#XF`, 20 `#VE`, 21 `#CP` | `Fault`: o processo morre. (`#NM` não deveria ocorrer: `CR0.TS = 0`, o estado de SIMD é eager) | Como hoje: relata e para |
| 14 `#PF` | Resolve (página zero, frame novo) ou `Fault` | Se o `pc` está no `usercopy`, trata como [falta do processo ou *fixup*](#cópia-usuáriokernel); senão como hoje |
| 1 `#DB` | `Fault` (sem depuração na fase 4). **Ganha IST própria** (IST4, com guarda abaixo), porque pode disparar na primeira instrução da entrada, antes do `swapgs` e da troca de pilha. **Vindo do anel 3, o stub da IST4 não trata nada nela:** faz `swapgs` (e o CR3 do kernel sob KPTI), **copia o quadro de hardware para a pilha da tarefa** (`gs:[kernel_rsp]`, a do `TSS.rsp[0]`) e segue como entrada normal, como a `idtentry_mce_db` do Linux ("switch stacks and treat it as a normal entry"). Matar o processo e o `schedule` rodam na pilha da tarefa, **nunca na IST** | Entrada paranoica na IST, relata e para |
| 2 NMI, 8 `#DF`, 18 `#MC` | Continuam com IST e fatais com relatório, mesmo vindos de usuário | idem |
| Qualquer outro vetor sem tratador | `Fault` | Como hoje |

**O `IRETQ` de saída pode falhar.** O Linux o diz no `entry_64.S`: "This may fault.
Non-paranoid faults on return to userspace are handled by fixup_bad_iret. These include
#SS, #GP, and #NP." Quando ocorre, a CPU já trocou `GS` e CR3 para os do usuário (o
`swapgs` e a troca de CR3 vêm **antes** do `iretq`), e o `CS` salvo é o do kernel: um stub
de exceção "do kernel" que confiasse no `CS` usaria o `GS` do usuário. O caminho
`.Lerror_bad_iret` do Linux faz `swapgs`, volta ao CR3 do kernel e chama `fixup_bad_iret`,
que monta o quadro "as if we faulted immediately after IRET" (**verificado**, cópia local).
Desenho do RustOS:

| O que | Como |
| --- | --- |
| Por construção | `CS` e `SS` são constantes do kernel; `RIP` vem de `thread_create`, validado (`USER_MIN <= entry < USER_TOP`), ou é o que a CPU salvou; `RFLAGS` vem da CPU ou, na saída de uma syscall, é `(R11 & 0x3C7FD7) \| 2` **com `NT` limpo** (o usuário põe `NT` e `AC` com `popf`) |
| O que sobra | O `RSP` do usuário (não canônico, que a CPU aceita em anel 3) e qualquer bit de `RFLAGS` que o `IRETQ` recuse. Que `IRETQ` faça `#GP` com `NT = 1`, e `#SS`/`#GP` com `RSP` não canônico, é **inferência a confirmar no SDM** ([lista](18_fase_4_decisoes_pedidas.md#fontes-lidas-e-o-que-ficou-por-ler)) |
| Onde se decide | **No stub dos vetores 11 `#NP`, 12 `#SS` e 13 `#GP`, em assembly, antes de tocar em `gs`:** compara o `RIP` do quadro com o endereço do `iretq` (`lea`/`cmp` relativos ao `rip`, que não usam `gs`). Igual: é o caso do *fixup*. Diferente: segue a entrada normal |
| *Fixup* | Só depois da comparação: `swapgs`; sob KPTI, o CR3 do kernel (o stub e a IDT estão no trampolim, então rodam com o CR3 do usuário); monta o `Fault` de usuário e o processo morre |
| Teste | `thread_create` com `entry`/`sp` em `0xFFFF_8000_0000_0000`, `0x0000_8000_0000_0000` e `USER_TOP`; usuário com `rsp` não canônico e uma IRQ; usuário que liga `NT = 1` e depois `AC = 1` por `popf` e faz syscall (o quadro sai sanitizado e o processo **não** morre, nem o kernel); e um `iretq` forçado a falhar em `#GP`, `#SS` e `#NP`, com e sem KPTI |

### aarch64: SVC

**Verificado** (Linux, `asm/esr.h`, `entry.S` e `arm_arch_timer.c`, lidos na pesquisa):

| Item | Valor |
| --- | --- |
| `ESR_ELx.EC` | bits 31:26; `IL` no bit 25; `ISS` nos bits 24:0 |
| SVC de AArch64 | `EC = 0x15` (`ESR_ELx_EC_SVC64`) |
| Outros `EC` que a fase usa | `0x20` (`IABT_LOW`, instrução vinda de EL inferior), `0x24` (`DABT_LOW`, dado), `0x00` (desconhecida), `0x18` (`SYS64`: acesso a registrador de sistema trapeado de EL0), `0x19` (SVE), `0x1D` (SME) |
| Vetores | 16 entradas, nesta ordem: EL1t, EL1h, EL0 de 64 bits, EL0 de 32 bits; cada uma com sync, IRQ, FIQ, SError, cada entrada com `.align 7` (128 bytes). Logo, **"EL inferior, AArch64" é o grupo 8 a 11** (offset `0x400`), por contagem do `entry.S` |
| Convenção ([syscall(2)](https://man7.org/linux/man-pages/man2/syscall.2.html)) | `svc #0`; número em `w8`; argumentos `x0`–`x5`; retorno `x0` e `x1` |
| Entrada de EL0 | Guarda `sp_el0` no quadro (`mrs x21, sp_el0`) e põe a tarefa em `sp_el0` (`msr sp_el0, tsk`); na saída, `msr sp_el0, x23` a partir do quadro e `eret` |
| KPTI | Troca o **`TTBR1_EL1`** (a metade do kernel) por uma tabela só com o trampolim, somando `TRAMP_SWAPPER_OFFSET` e ligando `USER_ASID_FLAG`; o `TTBR0_EL1` não muda |
| Pilha na pilha cheia | O `kernel_ventry` do Linux testa o estouro de pilha sem registrador livre (`add sp, sp, x0` e `sub x0, sp, x0`) e, na falha, guarda `x0` em `tpidr_el0`/`tpidrro_el0` e vai para a `overflow_stack` |
| Contador em EL0 | `arch_counter_set_user_access` liga `ARCH_TIMER_USR_VCT_ACCESS_EN` (acesso de EL0 ao contador virtual) e desliga os acessos ao contador físico, aos timers e ao *event stream* |

**NÃO VERIFICADO** (Arm ARM, DDI 0487, não lido): o layout do `ISS` do SVC (o `imm16`
estaria em `ISS[15:0]`; o desenho usa sempre `svc #0` e não lê o `imm`); `TCR_EL1.A1`,
`T0SZ`, `TG0` e os campos de cacheabilidade do `TTBR0`; a largura do ASID
(`ID_AA64MMFR0_EL1.ASIDBits`); a codificação de `AP[2:1]` para "EL0 pode acessar"; o
`nG` das folhas de usuário; o número do bit `EL0VCTEN` em `CNTKCTL_EL1` (o Linux o chama
de `USR_VCT_ACCESS_EN`); o efeito do PAN sobre `LDTR`/`STTR` e o do `UAO`; um `ERET` com
estado ilegal.

**Desenho (proposta):**

| Passo | O quê |
| --- | --- |
| Vetores 8 a 11 | Hoje só param ([DEBT-0012](../tech-debt/0012-excecao-de-usuario-so-para-a-cpu.md)). Passam a salvar o quadro na `SP_EL1` da tarefa (a exceção vinda de EL0 usa `SP_EL1`) e a despachar por `EC` |
| `SP_EL0` | Na entrada de EL0 o quadro guarda o `SP_EL0` do usuário; a saída o restaura. **Nada do kernel usa `SP_EL0` depois disso** (o kernel roda em EL1h) |
| Pilha de falhas | **O invariante "`SP_EL0` é sempre a pilha de falhas" é falso:** entre a entrada de EL0 e o prólogo que guarda `SP_EL0`, ele é o do usuário, e o `vector_fault` do [SDD 15](15_tarefas_e_preempcao.md) (`msr spsel, #0` e empilhar) empilharia na pilha do usuário. O desenho **adota a ordem do Linux**: toda exceção síncrona ou SError de EL1h entra **na pilha da tarefa** (`SP_EL1`), com o teste de estouro de pilha do `kernel_ventry` (**verificado**: `add sp, sp, x0` e `sub x0, sp, x0`, sem registrador livre, e `tbnz` no bit `THREAD_SHIFT`); **só no estouro** vai para a pilha de falhas (o `__bad_stack`, que guarda `x0` em `tpidr_el0`/`tpidrro_el0`). O teste por bit do `sp` exige a pilha alinhada ao dobro do tamanho: pilha de 32 KiB em 64 KiB de endereço (custo só de janela, não de memória), ou comparar o `sp` com o limite guardado na área por CPU |
| Por que a ordem muda | O `vector_fault` de hoje também serve o `brk` que **volta** (`exception.rs`, [SDD 10](10_interrupcoes.md)) e passa a servir a falta recuperável do `usercopy`. Quem volta não pode empilhar numa pilha de falhas que a próxima falha reinicia do topo. Com a ordem do Linux, `brk`, `usercopy` e IRQ usam o mesmo caminho: a pilha da tarefa |
| Falta aninhada | No `__bad_stack`, se o `sp` já está **dentro** da pilha de falhas (falta enquanto relata uma falta), **continua nela**, sem reiniciar do topo |
| `TPIDR_EL1` | O ponteiro da área por CPU (com o topo e a base da pilha de falhas) é posto em `exception::install`, **antes de qualquer falha possível**; o caminho de estouro o lê |
| Efeito nos outros capítulos | O [SDD 15](15_tarefas_e_preempcao.md) e o [SDD 10](10_interrupcoes.md) descrevem o `vector_fault` de hoje ("sempre e sem testar o `sp`"). Os dois ganham uma nota "desenho da fase 4" no mesmo commit deste capítulo; o texto de hoje só muda quando o código mudar |
| `TPIDR_EL0` e `TPIDRRO_EL0` | O usuário escreve `TPIDR_EL0`: salvo e restaurado por thread (ou zerado ao trocar de processo), senão vaza entre processos. O `Context` do aarch64 cresce 16 bytes |
| `CNTKCTL_EL1` | Liga o acesso de EL0 ao `CNTVCT_EL0` e desliga o resto, como o Linux. Sem isso, `mrs x, cntvct_el0` em EL0 dá `EC 0x18` e a medida da syscall nula não roda. É a contrapartida do `CR4.TSD = 0` do x86_64 ([SDD 09](09_seguranca.md)) |
| `TTBR0_EL1` de tarefa só de kernel | Uma **raiz vazia** estática, com ASID reservado: a tarefa do kernel não herda o `TTBR0` do último processo (nem lê memória dele por engano ou por especulação). No x86_64, a tarefa só de kernel usa a raiz do kernel, de metade baixa vazia, como hoje |
| Número e retorno | `x8`; fora da tabela devolve `NotSupported`. `x0` = status, `x1` = valor ([ABI](#abi-nativa-mínima)) |
| `eret` | `SPSR_EL1` com `EL0t`, `DAIF` limpo (IRQ ligada); `ELR_EL1` = entrada do programa |
| PAN e `UAO` | O `usercopy` usa `ldtr`/`sttr` (acesso não privilegiado) com `UAO = 0` fixo e o PAN ligado o tempo todo; se o Arm ARM mostrar que o PAN ainda barra `ldtr`/`sttr`, a rotina abre a janela do PAN em volta da cópia |

### Cópia usuário↔kernel

Toda leitura e escrita de memória de usuário passa por rotinas de `arch/<isa>/src/usercopy.rs`
(`asm!`), e nunca por desreferência.

| Camada | O quê |
| --- | --- |
| Pura, em `core/mm/src/user.rs` | `UserRange::new(addr, len) -> Result<_, _>`: recusa `len` que estoura o endereço e fim acima de `USER_TOP`. Testada no host. **Todo** argumento de ponteiro de syscall vira `UserRange` antes de qualquer acesso, mesmo com SMAP/PAN ligados |
| Uma leitura só | `copy_from_user` copia o argumento inteiro **para memória do kernel** (pilha ou `Vec`), e todo o resto do código valida e usa **a cópia**. O ponteiro de usuário nunca é relido: outra thread do processo pode mudar o conteúdo entre duas leituras (*double fetch*). Vale para o array de handles, o nome do processo, o corpo de uma mensagem |
| Sem lock do VMAR | A cópia **não** roda com o lock do espaço do processo preso: a falta durante a cópia precisa dele para resolver a página (o `IrqLock` não reentra). Ordem: copia, solta, e só então toma o lock para validar e agir |
| Falta durante a cópia | Primeiro o tratador a trata **como falta do processo**: se cai numa região com permissão, resolve (página zero, frame novo, COW) e **volta à instrução que faltou**. Só se não resolve (fora de região, sem permissão) usa o *fixup*: o `pc` está entre `__usercopy_start` e `__usercopy_end`, e o `pc` vira o ponto único de recuperação, que devolve os bytes que faltaram. A syscall responde `OutOfRange`; **não mata o processo** |
| Instruções | x86_64: `stac`/`clac` em volta; aarch64: `ldtr`/`sttr` (acima) |
| `unsafe` | Mora só nessas rotinas, com `// SAFETY:` ([guidelines 04](../guidelines/04_unsafe_e_asm.md)) |

### Interação com o SMP e o estado por CPU

| | x86_64 | aarch64 |
| --- | --- | --- |
| Registrador da área por CPU | `IA32_GS_BASE` no kernel; `IA32_KERNEL_GS_BASE` guarda o do usuário | `TPIDR_EL1` (uso do registrador como ponteiro por CPU: prática comum, **NÃO VERIFICADO** em fonte lida) |
| O que a área guarda | `user_rsp` (rascunho), `kernel_rsp` (topo da pilha da tarefa), id da CPU, tarefa corrente, pilha do trampolim (KPTI) | idem, sem `user_rsp` (o `SP_EL0` vai ao quadro); pilha de falhas |
| Quem a cria | A 2ª onda da fase 3 ([DEBT-0011](../tech-debt/0011-gic-e-timer-so-na-cpu-0.md), [DEBT-0036](../tech-debt/0036-escalonador-com-estado-de-troca-global.md), [DEBT-0039](../tech-debt/0039-escalonador-de-uma-cpu.md)): estado de troca por CPU | idem |
| Se a fase 4 vier antes do SMP | Cria o esqueleto para a CPU 0; o SMP só acrescenta instâncias. **Nada na fase 4 pode supor uma CPU** além do que o [SDD 15](15_tarefas_e_preempcao.md) já supõe | idem |
| TLB | Uma CPU: `invlpg`/`tlbi` local basta. Processo com threads em duas CPUs exige *shootdown* por IPI; **não existe** até o SMP ([SDD 08](08_roadmap.md)) | idem |

## Espaço do usuário

### Layout, igual nas duas ISAs

Uma constante só (47 bits) para as duas ISAs mantém o carregador, os testes e o
`core/mm` sem `cfg` por arquitetura. `USER_MIN`, `USER_TOP` e `PAGE_SIZE` moram em
`hal::abi`, porque `arch/`, `core/mm` e `core/elf` os usam. **Verificado** ([x86_64 mm](https://docs.kernel.org/arch/x86/x86_64/mm.html)): o
usuário do Linux vai de 0 a `0x00007fffffffefff` com 4 KiB de buraco em
`0x00007ffffffff000`. **Verificado** ([arm64 memory](https://docs.kernel.org/arch/arm64/memory.html)): `TTBR0` para o
usuário, `TTBR1` para o kernel, seleção pelo bit 55; faixa padrão de 48 bits.

| Faixa (proposta) | O quê | Sorteio |
| --- | --- | --- |
| `[0, 0x1_0000)` | **Faixa nula**: nada mapeia (o equivalente do `mmap_min_addr`; o valor do Linux **NÃO VERIFICADO**) | — |
| `0x40_0000` em diante | Imagem do `ET_EXEC`, no endereço em que o linker a pôs (`user/link/<isa>.ld`) | Nenhum na fase 4 ([decisão 5](18_fase_4_decisoes_pedidas.md#decisão-5--pie-de-usuário-agora-ou-depois)) |
| `[mmap_base, …)` | Mapeamentos pedidos com `ANY` | `mmap_base` com 28 bits de página |
| Janela da pilha | Pilha inicial, com uma página de guarda **abaixo** e outra **acima** | Topo com 28 bits de página |
| `USER_TOP = 0x0000_7FFF_FFFF_F000` | Fim exclusivo; a última página nunca mapeia | — |

Pilha inicial: 16 páginas (64 KiB), tamanho fixo, sem crescer sozinha. **Verificado**
([mmap(2)](https://man7.org/linux/man-pages/man2/mmap.2.html)): `MAP_GROWSDOWN` faz tocar a página de guarda crescer o mapeamento, e passado o
limite dá `SIGSEGV`. O RustOS não copia isso: estouro de pilha cai na guarda e é falha
de usuário ([falhas](#falhas-de-usuário)). `argc` é 0, `argv` e `envp` vazios.

Só o `init`, carregado pelo kernel, usa a janela de pilha dedicada. Um filho carregado
pelo `user/rt` mapeia a pilha como mais um mapeamento `ANY`, que cai na região do
`mmap_base`; as guardas são páginas que ele deixa sem mapear.

Os valores de `mmap_base` e da janela da pilha (por exemplo, base de mapeamento
num intervalo de 1 TiB acima de 64 GiB, janela da pilha de 1 TiB terminando em
`0x7000_0000_0000`) são constantes a fixar no PR de código.

### ASLR: quantos bits

| Fato | Valor | Fonte |
| --- | --- | --- |
| Piso do Linux x86_64 para a aleatoriedade do `mmap` | 28 bits (`ARCH_MMAP_RND_BITS_MIN`), teto 32 (`ARCH_MMAP_RND_BITS_MAX`), em 64 bits | **Verificado**: `arch/x86/Kconfig` (lido por ferramenta de busca; o valor padrão, definido fora daquele arquivo, **NÃO VERIFICADO**) |
| Piso do RustOS | 28 bits de página no `mmap_base` e 28 no topo da pilha: 2^28 páginas de 4 KiB = 1 TiB de intervalo, cabe nos 128 TiB de usuário | **Proposta** |

O sorteio é **por região, não por mapeamento**: o `ANY` aloca em sequência a partir do
`mmap_base`. Sortear cada mapeamento no espaço inteiro espalharia as tabelas de página
(até 3 tabelas novas, 12 KiB, por mapeamento isolado), o que pesa na meta de memória
(**inferência**, não medida). A semente vem de `random::next_u64`
([SDD 09](09_seguranca.md)): **em CPU sem RDSEED/RDRAND/RNDR o ASLR herda o jitter
fraco** ([DEBT-0019](../tech-debt/0019-aleatorio-sem-fonte-de-hardware-fraco.md); no
Mac com HVF a CPU não anuncia `RNDR`). O boot relata a fonte e o PR de código mede a entropia efetiva
de 5 000 processos (teste de propriedade com o gerador próprio, como em `core/sched`).

### Página zero compartilhada e COW

| Caso | O que acontece |
| --- | --- |
| Leitura de página anônima nunca tocada | A tabela aponta para **um** frame zerado, permanente, só leitura e NX. Nunca vai ao buddy, então não precisa de contagem de referência |
| Escrita nessa página | Falta de proteção: aloca frame, zera, troca a entrada para o frame novo com escrita. É o COW a partir da página zero |
| Escrita em página anônima ausente | Aloca frame, zera e mapeia (o caso quente; o zeramento é o do [DEBT-0020](../tech-debt/0020-zerar-pagina-pelo-memset-generico.md)) |
| COW de página já privada (`fork`, `vmo_create_child`) | **Fora da fase 4** (a ABI nativa não tem `fork`; vem com a fase 5) |

O comportamento do Linux (leitura de anônimo mapeia a página zero; escrita copia) está
no roadmap como "validar em `mm/memory.c`": **NÃO VERIFICADO**. A pesquisa não achou a
fonte primária. Ler `do_anonymous_page` e `wp_page_copy` antes de codar, e dizer aqui o
que difere. A página de um VMO físico (initramfs, framebuffer) nunca é zero nem COW.

### O trampolim e o caminho pronto para KPTI

**Verificado** ([Linux PTI](https://docs.kernel.org/arch/x86/pti.html)): as tabelas de usuário mapeiam só o necessário para entrar e sair (entrada e saída, IDT,
`cpu_entry_area` com a TSS por CPU); as tabelas do kernel mapeiam tudo, com NX na parte
do usuário para pegar um CR3 esquecido. Custo segundo a doc: uma troca de CR3 a cada
entrada e a cada saída, "roughly a hundred cycles each"; o `fork` duplica os dois PGDs;
perda de TLB "never exceed[s] 1%" (afirmação da doc, sem hardware citado). Sem PCID,
cada escrita no CR3 esvazia o TLB inteiro.

"Pronto para KPTI" quer dizer que a **estrutura** existe desde a 4a, sem o custo, e a
4e só liga:

| Peça | Fatia | O quê |
| --- | --- | --- |
| Seção de entrada | 4a | Todo stub de vetor, a entrada de syscall, a GDT, a IDT e a TSS vivem em `.entry.text`/`.entry.data`, alinhadas a 4 KiB, no linker script. Sem KPTI são só mais uma seção |
| Pilha de entrada por CPU | 4a | Uma pilha pequena por CPU na área por CPU. A saída por `IRETQ`/`eret` passa por ela, como o Linux faz (`TSS_sp0` é a pilha do trampolim na saída) |
| Raízes | 4e | **x86_64:** duas por processo: a *raiz de kernel* (metade de usuário + kernel inteiro) e a *raiz de usuário* (a mesma metade de usuário + uma entrada de nível alto com o trampolim). As duas compartilham as subárvores da metade baixa. **aarch64:** uma raiz de usuário por processo (`TTBR0`) e **uma só** tabela de trampolim para a máquina (`TTBR1`), como o Linux |
| A troca | 4e | Um teste de byte (`kpti_on`) na entrada e na saída, não remendo de código (o W^X do kernel torna o remendo caro). Custo de um desvio previsível, a medir |
| Relatório | 4e | `[rustos] mitigations: kpti=<on\|off> reason=<…>` no boot |

**Quem é vulnerável.** **Verificado** (Linux, `cpu/common.c`): o `X86_BUG_CPU_MELTDOWN`
liga a menos que a CPU esteja na lista `NO_MELTDOWN` ou o `IA32_ARCH_CAPABILITIES`
tenha `RDCL_NO` (bit 0 do MSR `0x10A`, **verificado** em `msr-index.h`). O conteúdo da
lista e o bit do `CPUID` que anuncia o MSR **NÃO VERIFICADO**. Para aarch64 a lista de
CPUs com Meltdown **NÃO VERIFICADO**. Política da proposta: **sem
informação, vulnerável** (KPTI ligado).

**A metade do kernel em todo processo.** Cada raiz de processo copia as 256 entradas
altas da raiz do kernel (as subárvores são compartilhadas por ponteiro). Isso só
funciona se **toda entrada de nível alto do kernel já existir antes do primeiro
processo**: o desenho as pré-cria no boot (janelas de dispositivos e de pilhas inclusive)
e o `check_wx` passa a conferir que a metade alta de cada raiz de processo é igual à do
kernel. Entrada nova depois do boot dessincronizaria as raízes (o Linux resolve com
`vmalloc_fault`; o RustOS evita pelo desenho).

### `CR3` e `TTBR0` por processo, com PCID e ASID

| | x86_64 | aarch64 |
| --- | --- | --- |
| Raiz do processo | `CR3` | `TTBR0_EL1` (e `EPD0` volta a 0 com `T0SZ` de 47 bits: **NÃO VERIFICADO** os campos, ler o Arm ARM) |
| Etiqueta | PCID nos 12 bits baixos do CR3 (`CR4.PCIDE`); **Verificado** que a CPU anuncia `PCID` e o Linux liga `PCIDE` (`cr4_init`); o bit 63 de "não esvaziar" e `INVPCID` **NÃO VERIFICADO** (SDM vol. 3A, "Process-Context Identifiers") | ASID em `TTBR0_EL1[63:48]` quando `TCR_EL1.A1 = 0`; largura de 8 ou 16 bits: **NÃO VERIFICADO** |
| Troca de processo | Escreve CR3 com o PCID do processo; sem esvaziar o TLB | Escreve `TTBR0_EL1` com o ASID; sem `tlbi` |
| Alocação | Gerador global de ASID/PCID por CPU, com contador de geração: esgotou, esvazia tudo e começa outra geração. Módulo puro em `core/mm/src/asid.rs`, testado no host | idem |
| Com KPTI | Dois PCIDs por processo (raiz de kernel e raiz de usuário): o custo é metade do espaço de PCID | O ASID já distingue; o `USER_ASID_FLAG` do Linux marca a variante de usuário |
| Fallback | Sem PCID, todo CR3 esvazia o TLB: correto e mais lento | Sem ASID útil, `tlbi` por troca |
| `PageFlags` | `user` liga U/S (bit 2) | `user` liga o acesso de EL0 e `nG`; o kernel segue com UXN e PXN |

Folhas de usuário: x86_64 `U/S = 1`, NX salvo se a permissão for execução; com SMEP o
kernel não executa, com SMAP não acessa sem `AC`. aarch64: código de usuário com
`PXN = 1, UXN = 0`; dado de usuário com `UXN = 1`. **O W^X do usuário já é do mapeador**
(`MapError::WriteExecute`): `vmar_map` e as permissões de `vmar` recusam W+X.

## Mitigações de CPU

O [SDD 08](08_roadmap.md) e o [SDD 09](09_seguranca.md) prometem estas mitigações para a
fase 4, porque mexem na entrada e na saída de syscall. Todas entram na **4e**, com o custo
medido ([SDD 07](07_desempenho_e_recursos.md)); a regra de cada uma é "liga onde a CPU
precisa, relata no boot". As fontes de cada uma estão no SDD 09; a pesquisa desta fase
**não** leu as páginas do Linux sobre elas (`spectre.html`, `mds.html`), então o que
segue sobre condições e custos é **NÃO VERIFICADO** até a leitura.

| Mitigação | Contra | Onde entra | Custo |
| --- | --- | --- | --- |
| Clamp de índice (Spectre v1) | Leitura especulativa fora da tabela | O número da syscall (índice da tabela de despacho) e o índice do handle na tabela do processo: `idx & máscara(idx < len)`, sem desvio. **Sempre ligado**, em toda CPU | Duas ou três instruções por uso; medir. A técnica (o `array_index_nospec` do Linux) **NÃO VERIFICADO** em fonte lida |
| IBPB | Spectre v2 entre processos | Na troca para outro processo, se a CPU anuncia a instrução e é vulnerável | A medir (o SDD 09 não tem número na fonte) |
| eIBRS ou retpoline | Spectre v2 no kernel | eIBRS onde a CPU tem; retpoline onde não; o kernel é compilado soft-float, o retpoline exige conferir o alvo de build | A medir |
| STIBP | Spectre v2 entre *hyperthreads* | Só com SMT, e só se a política pedir | A medir |
| `VERW` na saída para usuário | MDS | Se a CPU é vulnerável e não anuncia `MDS_NO`; o Linux tem `CLEAR_CPU_BUFFERS` antes do `sysretq` (**verificado** no `entry_64.S`) | A medir |
| SSBD | Spectre v4 | Por processo, sob pedido (política); padrão desligado | A medir |
| KPTI | Meltdown | [Trampolim](#o-trampolim-e-o-caminho-pronto-para-kpti) | ~100 ciclos por troca de CR3 |

O Linux faz na entrada `IBRS_ENTER`, `UNTRAIN_RET` e `CLEAR_BRANCH_HISTORY`
(**verificado**: estão no `entry_64.S`); o desenho não os copia sem a leitura da página de
cada um.

**O que não puder ser exercitado fora de metal** (a QEMU e o HVF não modelam `IBPB`, `VERW`
nem as MSRs de controle de especulação de forma que sirva) **vira dívida com gatilho "o
primeiro boot em metal"**, registrada no PR da 4e, com a linha `mitigations:` dizendo
`off (untested)` para cada uma. O que se pode medir em KVM, HVF ou TCG entra com número.

## Modelo de objetos e handles

**Modelo A, decidido** ([SDD 09](09_seguranca.md)): no núcleo, só handles com direitos que
só diminuem, sem autoridade ambiente; nenhuma API do núcleo recebe uid. O desenho
segue o Zircon, com as páginas **verificadas** abaixo. O que não foi lido do Zircon
(valores numéricos de direitos, assinaturas completas, o comportamento de uma ponta que
morre com mensagens na fila) **não é premissa**.

| Fato verificado | Fonte |
| --- | --- |
| Direitos (DUPLICATE, TRANSFER, READ, WRITE, MAP, EXECUTE, WAIT, INSPECT, GET/SET_PROPERTY, DESTROY, MANAGE_PROCESS, MANAGE_THREAD) e a possibilidade de reduzi-los por `zx_handle_replace` e `zx_handle_duplicate` | [rights](https://fuchsia.dev/fuchsia-src/concepts/kernel/rights) |
| `zx_handle_replace(handle, rights, out)`: direitos novos devem ser subconjunto (senão `ZX_ERR_INVALID_ARGS`); **"handle is always invalidated"**, em sucesso e em erro; direitos exigidos: **"None"**; erros `BAD_HANDLE`, `INVALID_ARGS` e `NO_MEMORY` | [handle_replace](https://fuchsia.dev/fuchsia-src/reference/syscalls/handle_replace) |
| `zx_channel_write`: **"All handles are discarded and no longer available to the caller, on success or failure"**; o canal exige `WRITE` e cada handle do array exige `TRANSFER`; handle repetido no array dá `BAD_HANDLE`; a **própria ponta** do canal no array dá `NOT_SUPPORTED`; limites: 65 536 bytes e 64 handles por mensagem | [channel_write](https://fuchsia.dev/fuchsia-src/reference/syscalls/channel_write) |
| `zx_process_create(job, name, …)` devolve o handle do processo e o do VMAR raiz; a thread só roda após `zx_process_start`; exige `ZX_RIGHT_MANAGE_PROCESS` no job | [process_create](https://fuchsia.dev/fuchsia-src/reference/syscalls/process_create) |
| `zx_channel_create`: dois handles; mensagens de um lado são lidas no outro; transferência de handle com `ZX_RIGHT_TRANSFER`. O comportamento de *peer closed* **NÃO VERIFICADO** na página | [channel_create](https://fuchsia.dev/fuchsia-src/reference/syscalls/channel_create) |
| seL4: Copy (com direitos), Mint (direitos e badge), Move, Mutate, Delete, Revoke (apaga filhos); Untyped retype: memória vira objeto só por capability | [API do seL4](https://docs.sel4.systems/projects/sel4/api-doc.html) (a definição geral dos direitos Read, Write, Grant, GrantReply não foi lida) |

### Os objetos da fase

| Objeto | O que é | Quem o cria |
| --- | --- | --- |
| **Process** | Espaço de endereçamento, tabela de handles, threads, canal de eventos | `process_create`; o kernel cria o do `init` |
| **Thread** | Uma tarefa do escalonador com contexto de usuário, pilha de kernel e área de SIMD | `thread_create` |
| **VMO** (objeto de memória virtual) | Um conjunto de páginas: anônimas e preguiçosas, ou de frames físicos já existentes (initramfs) | `vmo_create`; o kernel cria os físicos |
| **VMAR** (região de endereço) | Um intervalo do espaço de um processo, com as permissões máximas que mapeamentos sob ele podem ter | O VMAR raiz vem com `process_create`; sem sub-VMARs na fase 4 |
| **Channel** | Dois handles pares; fila limitada de mensagens com bytes e handles | `channel_create` |
| **Console de depuração** | Escrita no console do kernel (serial e tela) | Só o kernel; vai ao `init` como handle |

### Tabela de handles

Proposta, sem equivalente verificado no Zircon:

| Item | Desenho |
| --- | --- |
| Valor | `u32` = `geração << 16 \| (índice + 1)`. **`0` nunca é válido** (o índice mais um é no mínimo 1). A geração começa em 0, então o primeiro handle de um processo vale `1` |
| Geração | 16 bits, sobe a cada reuso do slot. **Aposentadoria:** o slot cuja geração chegaria a `0xFFFF` é aposentado e nunca mais é usado, então a geração nunca dá a volta e handle velho jamais aponta para outro objeto. Teste de propriedade: nenhum valor de handle reaparece |
| **Orçamento da geração** | Um valor de 32 bits tem 2^32 combinações: **cada processo gasta, na vida inteira, até cerca de 2^32 handles** (65 534 slots × 65 535 usos); depois disso a tabela não tem slot e toda criação dá `NoMemory`. A 1 M de handles por segundo são cerca de 71 minutos: um servidor que cria um handle por pedido esgota. A saída é **contrato** e vai para a [decisão 2](18_fase_4_decisoes_pedidas.md#decisão-2--abi-nativa-lista-números-nomes-e-contrato): lista livre FIFO (**não** aumenta o total, só o gasta por igual), ou **handle de 64 bits antes de congelar a ABI** |
| Teto | 4 096 handles **em tabela** por processo (proposta); passar disso é `NoMemory`. O índice vai até 65 534. **O handle dentro de uma mensagem em fila não conta nesses 4 096** (por isso há também o [limite de canais](#destruição-de-um-canal)) |
| Slot | **24 bytes**, não 16. `Object` é um enum fechado de `Arc<Process>`, `Arc<Thread>`, `Arc<Vmo>`, `Arc<Vmar>`, `Arc<ChannelEnd>` e `Arc<Console>`: tag mais ponteiro dão **16 bytes** (medido com `rustc` na revisão); somam-se `rights: u32` e `generation: u16`, e o alinhamento fecha em 24. Chegar a 16 exigiria `tag: u8` e ponteiro como campos separados, com `unsafe` para remontar o `Arc` a cada acesso: **sem ganho medido, fica 24**. **Sem `dyn`** em todo caso: `Arc<dyn Objeto>` tem ponteiro gordo (16) e o slot passa de 24. Pior caso do teto: 4 096 × 24 = 96 KiB por processo, e a tabela cresce por demanda |
| Tabela | `Vec` de slots mais lista de livres, cresce por demanda; protegida por **um `IrqLock` por processo** |
| Referências | O `Arc` segura a **memória** do objeto (handle, mapeamento e fila de mensagem); o **ciclo de vida lógico** vem da contagem de handles, abaixo. Filho para pai usa `Weak`: sem ciclo de `Arc` |
| Contagem de handles | Cada objeto guarda `handle_count`: handles em tabela **e em trânsito** (dentro de uma mensagem). Chega a zero: o objeto recebe `on_zero_handles()` |
| `on_zero_handles` | Canal: fecha a ponta ([abaixo](#destruição-de-um-canal)). VMO: nada (os mapeamentos o seguram; os frames voltam quando o último `Arc` solta). Process e Thread: nada (vivem até sair) |

#### Destruição de um canal

A destruição é por **contagem de handles**, não pelo último `Arc`.

| Passo | O quê |
| --- | --- |
| 1 | Quando a contagem de uma ponta chega a zero, ela **morre**: esvazia a fila **dela** (as mensagens destinadas a ela) e fecha os handles que estão nessas mensagens, em laço com lista de trabalho (sem recursão: fechar um handle pode matar outra ponta, que esvazia outra fila) |
| 2 | Levanta `PEER_CLOSED` na outra ponta. As mensagens que já estão na fila da outra seguem legíveis: quem escreve e fecha ainda é lido |
| 3 | `PEER_CLOSED` depende dos **handles** da outra ponta, não de `Arc` |

**O ciclo.** O processo tem `h_A` e `h_B`, escreve `h_B` por `h_A` e fecha `h_A`. A mensagem
com `h_B` fica na fila de B; `h_B` conta como handle em trânsito, então a contagem de B é 1
e nunca chegaria a zero: ninguém pode ler a fila de B, e ela segura B. Cada ponta guarda
`próprios_em_minha_fila`, o número de handles **dela mesma** que estão na fila **dela**.
Quando `handle_count == próprios_em_minha_fila`, a ponta é **inalcançável** e morre pelo
passo 1, o que fecha o `h_B` da mensagem e quebra o ciclo.

**Quando a checagem roda: sempre que um dos dois números muda**, não só no
`on_zero_handles`. Ela roda (a) quando `handle_count` cai (um `handle_close`, uma
transferência), e (b) **quando `próprios_em_minha_fila` sobe, isto é, na escrita**: no
cenário acima, o `channel_write` que põe `h_B` na fila de B é o que deixa B inalcançável
(a contagem de B é 1, a de próprios é 1), e B morre **na própria escrita**, não no `close`
de `h_A`. **Testes obrigatórios** no host (`core/object`): o cenário nas duas ordens
(escrever e fechar; fechar a outra ponta antes de escrever).

**Vazamento entre filas: ilimitado, e o teto não o limita.** Ciclos entre duas ou mais
filas (um handle de B1 dentro da fila de B2 e um de B2 dentro da de B1) **não** são
coletados. O teto de 16 mensagens e 8 handles limita o **tamanho** de cada ciclo, não o
**número** de ciclos; e o handle em fila não conta nos 4 096 da tabela. Um programa que
crie ciclos em laço vaza sem fim.

| Opção | Custo |
| --- | --- |
| Coletar por alcançabilidade (marcar a partir dos handles em tabela) | Percorre todos os canais; é um coletor de lixo no kernel. Mais código e mais tempo de pausa |
| **Limitar canais e objetos por processo (escolhida para a fase 4)** | Um contador por processo: `channel_create` e `vmo_create` somam, a morte do objeto subtrai; passar de `MAX_CHANNELS` (proposta: 256 pontas) ou de `MAX_VMOS` é `NoMemory`. A ponta continua cobrada ao **criador** enquanto existir, esteja o handle onde estiver (tabela ou fila) |

O limite **não fecha o problema**: um ciclo cobrado a um processo que já morreu segue vivo
(a cobrança some, o vazamento fica), e muitos processos que criam e morrem somam. O que
sobra é **dívida da 4c** (a coleta por alcançabilidade), registrada com o gatilho "o
primeiro programa que produza o ciclo, ou os grupos de recursos da fase 9".

### Operações sobre handle

**Atomicidade.** Validar o handle e removê-lo da tabela é **um passo só, sob o lock da
tabela**: entre ler o valor e removê-lo, nenhuma outra thread do processo pode fechá-lo,
substituí-lo ou enviá-lo. Duas threads que fecham o mesmo handle: uma vê `BadHandle`. Vale
para todas as linhas abaixo.

**Regra geral para toda chamada que move handle** (`channel_write`, e `process_create` com
`bootstrap` e `events`): (1) o handle que **autoriza** a chamada (`ch`, `parent`) é validado
primeiro, e erro nele **não consome nada**; (2) os handles **movidos** são tratados como um
array: validar **todos**, depois remover **todos** de uma vez, sob o lock da tabela;
(3) o mesmo valor duas vezes (`bootstrap == events`, ou repetido no array) é `BadHandle`;
(4) a partir da remoção, em erro, os removidos são **fechados**; (5) o handle que o kernel
guarda (o `events`) entra em `handle_count` **na criação** do processo, senão a ponta
morreria ali.

| Operação | Regra |
| --- | --- |
| `handle_close` | Remove o slot. Não pede direito. Se foi o último handle do objeto, `on_zero_handles` roda **depois** de soltar o lock da tabela |
| `handle_duplicate(h, rights)` | `h` exige `DUPLICATE`; `rights ⊆ h.rights`; insere o handle novo e devolve o valor |
| `handle_replace(h, rights)` | Remove `h` **sempre** que ele existe, em sucesso e em erro (**verificado** no Zircon: "handle is always invalidated"). Se `rights ⊄ h.rights`: `InvalidArgs`, com o original já invalidado. `h` que não existe: `BadHandle` (nada a invalidar). **Nenhum direito é exigido** (**verificado**: "Rights: None"); em particular, não pede `DUPLICATE` |
| `process_create(parent, …, bootstrap, events)` | `parent` com `MANAGE_PROCESS` (erro: nada consumido). `bootstrap` (`TRANSFER`) e `events` (`WRITE` e `TRANSFER`) seguem a regra geral. O `bootstrap` vai para a tabela do filho; o `events` fica **no Process**, contado em `handle_count`, e o kernel o fecha depois de escrever o evento final (então o pai vê `PEER_CLOSED` depois do evento) |
| `channel_write(ch, bytes, handles)` | Abaixo |
| `channel_read(ch, …)` | Abaixo |

**`channel_write`**, na ordem (o array de handles já foi **copiado** para o kernel, ver
[cópia usuário↔kernel](#cópia-usuáriokernel)). A ordem de locks é **tabela, depois canal**,
nunca o contrário:

1. Sob o lock da tabela, valida `ch`: existe, é canal, tem `WRITE`. Se não: erro **e nenhum handle consumido** (a página do Zircon não distingue esse caso: **NÃO VERIFICADO**; o RustOS poupa quem errou o handle do canal).
2. **Sob o lock do canal, antes de tirar qualquer handle da tabela:** a ponta oposta está viva? Se não, `PeerClosed` (a falha **consome** o array, passo 5). Há espaço na fila dela? Se não, **`ShouldWait`: nada é consumido** e nenhum estado muda. Havendo espaço, **reserva o slot** da mensagem.
3. Para cada handle do array: existe (`BadHandle` se não); não repete no array (`BadHandle`); tem `TRANSFER` (`AccessDenied`); não é a **própria ponta** de `ch`, isto é, nem `ch` nem outro handle da mesma ponta (`NotSupported`). Conferido pela identidade da ponta, não pelo valor do handle.
4. **Remove da tabela todos os handles do array que existem**, no mesmo passo; sem erro, a mensagem ocupa o slot reservado e `próprios_em_minha_fila` e a [checagem do ciclo](#destruição-de-um-canal) rodam.
5. Com erro **depois** do passo 2 (inclui `PeerClosed` e os erros do passo 3): os handles removidos são **fechados**, como no Zircon ("discarded … on success or failure"), e a reserva é solta. **`ShouldWait` fica de fora desse descarte**: é uma condição de repetir, não uma falha da mensagem. Consequência: uma fila cheia esconde um erro do array até a repetição.

**`channel_read`**, na ordem. A falha da cópia para o usuário **não perde mensagem nem handle**:

1. Sob o lock do canal, **espia** a mensagem da frente sem retirá-la, anotando o número de sequência dela. Fila vazia: `ShouldWait` (ou `PeerClosed`, se a ponta oposta morreu). Buffer menor que a mensagem: `BufferTooSmall`, com os tamanhos em `actual` e a mensagem **na fila**.
2. Sob o lock da tabela, **reserva** os slots dos handles da mensagem (`NoMemory` se faltar; a mensagem fica) e calcula os valores deles.
3. **Sem lock nenhum**, copia os bytes e os valores dos handles para o usuário (`copy_to_user`, que pode faltar e resolver página).
4. Se a cópia falhou: libera as reservas, devolve `OutOfRange`, e a mensagem **continua na fila**.
5. Se deu certo, sob os dois locks: confere que a mensagem da frente ainda tem o mesmo número de sequência (outra thread pode tê-la lido: então repete desde o passo 1), **retira-a** e instala os handles nos slots reservados.

**Invariantes e testes** (em `core/object`, host, `forbid(unsafe_code)`):

| Teste | O que prova |
| --- | --- |
| Direitos só diminuem | Sequências aleatórias de duplicar, substituir e transferir nunca produzem handle com direito fora do conjunto do handle raiz |
| **Entrelaçamento** | Cada operação é partida em passos (adquirir o lock, validar, remover, inserir) e o teste embaralha, de modo determinístico, os passos de duas ou três threads do mesmo processo sobre os mesmos handles. Nenhum handle é usado duas vezes, nenhum some sem ser fechado, e a soma dos `handle_count` é igual aos handles em tabela mais os em trânsito. **Sem o lock, o teste tem de falhar** |
| Ciclo da ponta | O cenário acima termina com as duas pontas mortas e `handle_count` zero, nas duas ordens |
| Atomicidade do `channel_write` | Handle repetido, a própria ponta, sem `TRANSFER` e `PeerClosed` consomem o array por inteiro; `ch` inválido não consome nada |
| **`ShouldWait` não consome** | Encher a fila de uma ponta; escrever com um handle: `ShouldWait`, e o handle **segue na tabela**; esperar `WRITABLE`; reescrever: sucesso, e o handle sai da tabela |
| `process_create` | `bootstrap == events` é `BadHandle` e consome os dois; o `events` entra em `handle_count` (a ponta do pai não vê `PEER_CLOSED` na criação); erro em `parent` não consome |
| `channel_read` | Cópia que falha (ponteiro ruim no meio do buffer) deixa a mensagem na fila e a tabela intacta; duas threads lendo o mesmo canal não perdem nem duplicam mensagem |
| Ciclo, na escrita | Escrever o handle da ponta oposta pela própria ponta mata a oposta já na escrita |
| Geração | Nenhum valor de handle reaparece, inclusive depois da aposentadoria do slot |

### Direitos iniciais ([decisão 3](18_fase_4_decisoes_pedidas.md#decisão-3--o-conjunto-inicial-de-bits-de-direito))

`rights` é `u32`. Bits de 11 a 31 reservados: valor com bit reservado ligado é
`InvalidArgs`.

| Bit | Nome | Sentido |
| --- | --- | --- |
| 0 | `DUPLICATE` | Pode duplicar |
| 1 | `TRANSFER` | Pode ser enviado por canal |
| 2 | `READ` | Canal: ler mensagens. VMO: mapear com leitura. VMAR: teto de leitura |
| 3 | `WRITE` | Canal e console: escrever. VMO: mapear com escrita. VMAR: teto de escrita |
| 4 | `EXECUTE` | VMO: mapear com execução. VMAR: teto de execução |
| 5 | `MAP` | VMO: pode ser mapeado. VMAR: pode mapear e desmapear sob ele |
| 6 | `WAIT` | Pode esperar sinais |
| 7 | `INSPECT` | Reservado para informação do objeto (nenhuma syscall da fase a usa) |
| 8 | `DESTROY` | Process: pode matar |
| 9 | `MANAGE_PROCESS` | Process: pode criar processo filho |
| 10 | `MANAGE_THREAD` | Process: pode criar thread nele |

Sem `vmo_read`/`vmo_write`: quem preenche um VMO o mapeia em si mesmo. Por isso
`READ`/`WRITE`/`EXECUTE` num VMO querem dizer "com que permissão pode ser mapeado". O
W^X é independente dos direitos: mapear com `WRITE|EXECUTE` é recusado mesmo que o
handle tenha os dois.

| Objeto | Direitos que fazem sentido |
| --- | --- |
| Process | `DUPLICATE`, `TRANSFER`, `WAIT`, `INSPECT`, `DESTROY`, `MANAGE_PROCESS`, `MANAGE_THREAD` |
| Thread | `DUPLICATE`, `TRANSFER`, `WAIT`, `INSPECT` |
| VMO | `DUPLICATE`, `TRANSFER`, `READ`, `WRITE`, `EXECUTE`, `MAP`, `INSPECT` |
| VMAR | `DUPLICATE`, `TRANSFER`, `READ`, `WRITE`, `EXECUTE`, `MAP` |
| Channel | `DUPLICATE`, `TRANSFER`, `READ`, `WRITE`, `WAIT` |
| Console | `DUPLICATE`, `TRANSFER`, `WRITE` |

### Autoridade que não passa por handle

"Sem autoridade ambiente" tem um piso: o que só afeta o próprio processo ou cria
objeto próprio.

| Chamada | Por que não pede handle |
| --- | --- |
| `process_exit`, `thread_exit` | Só afetam quem chama |
| `handle_close`, `handle_duplicate`, `handle_replace` | Agem na própria tabela |
| `clock_get_monotonic` | Tempo não é segredo (`rdtsc` já está liberado em usuário; [SDD 09](09_seguranca.md)) |
| `vmo_create`, `channel_create` | Criam objeto novo do chamador. **`vmo_create` gasta memória do sistema**: sem handle de recurso, um processo esgota a RAM. Na fase 4 a defesa é a contabilidade da [decisão 9](18_fase_4_decisoes_pedidas.md#decisão-9--overcommit-na-abi-nativa); o limite **por processo** é da fase 9, e até lá um processo pode reservar a RAM inteira e negar `NoMemory` aos outros |
| `random_get` | Entrega bytes do gerador do kernel, como o `getrandom` do Linux. Segredo do kernel não sai: o gerador troca a chave a cada pedido ([SDD 09](09_seguranca.md)) |

Processo nasce com **um** handle: o canal de boot.

## ABI nativa mínima

Tudo desta seção é **Proposta**; a numeração, os nomes e o contrato são a
[decisão 2](18_fase_4_decisoes_pedidas.md#decisão-2--abi-nativa-lista-números-nomes-e-contrato).

### Convenção de chamada

| | x86_64 | aarch64 | Fonte |
| --- | --- | --- | --- |
| Instrução | `syscall` | `svc #0` | **Verificado** ([syscall(2)](https://man7.org/linux/man-pages/man2/syscall.2.html)) |
| Número | `rax` | `x8` | idem |
| Argumentos 1 a 6 | `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9` | `x0` a `x5` | idem |
| Retorno | `rax` = status, `rdx` = valor | `x0` = status, `x1` = valor | Os dois registradores de retorno existem nas duas ISAs ([syscall(2)](https://man7.org/linux/man-pages/man2/syscall.2.html): `rax`/`rdx`, `x0`/`x1`) |
| Preservados | Todos, exceto `rax`, `rdx` e **`rcx`/`r11`** (o hardware os sobrescreve) | Todos, exceto `x0`, `x1` | `rcx`/`r11`: **verificado** no `entry_64.S` |
| Status | `0` = `Ok`; negativo = erro | idem | **Proposta** (o Linux devolve `-errno`; o espaço de números não é copiado) |

Dois valores de retorno evitam ponteiro de saída nas chamadas que criam objeto:
`vmo_create` devolve o handle em `rdx`/`x1`; `channel_create` e `process_create`
devolvem **dois** handles de 32 bits empacotados (`h0 | h1 << 32`).

### Entrada do programa

O contrato de como um programa **começa**, igual para o `init` e para qualquer filho.

| Item | x86_64 | aarch64 |
| --- | --- | --- |
| Modo | CPL 3; `CS` e `SS` de usuário; `RFLAGS = 0x202` | EL0t; `DAIF` limpo |
| `arg0`, `arg1` | `rdi`, `rsi` (os argumentos de `thread_create`) | `x0`, `x1` |
| Handle de boot | `process_create` **move** o `bootstrap` para a tabela do filho como o **primeiro handle, de valor `1`**. Quem chama `thread_create` passa `arg0` (o `user/rt` passa `1`); para o `init`, o kernel põe `arg0 = 1` | idem |
| Demais registradores | Zero | Zero |
| Pilha | `rsp = sp` de `thread_create`, alinhado a 16 (o alinhamento exato na entrada: **NÃO VERIFICADO** no psABI) | `SP_EL0 = sp`, alinhado a 16 |
| Conteúdo da pilha | O `init`: o kernel monta `argc = 0`, `argv`, `envp` e o [auxv](#auxv-mínimo). Filho: quem o criou (o `user/rt` do pai) monta o que quiser; o `user/rt` do filho lê o auxv se houver | idem |
| SIMD | Estado inicial do [SDD 15](15_tarefas_e_preempcao.md): `FCW` `0x037F`, `MXCSR` `0x1F80` | V0–V31, `FPSR`, `FPCR` zerados |
| TLS | Nenhum: `FS`/`GS` base em zero | `TPIDR_EL0` zero |

### Códigos de erro

| Código | Valor | Quando |
| --- | --- | --- |
| `Ok` | 0 | Sucesso |
| `InvalidArgs` | -1 | Argumento fora do domínio, bit reservado, tamanho zero, desalinhado, `entry`/`sp` fora de `[USER_MIN, USER_TOP]`, direitos que não são subconjunto |
| `BadHandle` | -2 | Valor que não é handle vivo (inclui geração velha) ou handle repetido num array |
| `WrongType` | -3 | Handle de outro tipo de objeto |
| `AccessDenied` | -4 | Falta direito no handle |
| `NoMemory` | -5 | Sem frame, sem slot, passou o teto de handles, ou a contabilidade negou ([decisão 9](18_fase_4_decisoes_pedidas.md#decisão-9--overcommit-na-abi-nativa)) |
| `BadState` | -6 | **Só** processo já morto ou em morte (`thread_create`, `process_kill`). Não existe "thread já iniciada": `thread_create` cria **e inicia** |
| `NotSupported` | -7 | Número de syscall desconhecido; flag que a fase não implementa; a própria ponta do canal no array de handles |
| `OutOfRange` | -8 | Ponteiro ou intervalo fora de `UserRange`, ou na faixa nula |
| `PeerClosed` | -9 | O outro lado do canal sumiu (na leitura, só depois de a fila esvaziar) |
| `ShouldWait` | -10 | Canal sem mensagem (leitura) ou cheio (escrita): espere e tente de novo. **Não consome handle** |
| `TimedOut` | -11 | `object_wait_one` chegou ao prazo |
| `BufferTooSmall` | -12 | Mensagem maior que o buffer: o tamanho vai em `actual` |

Falha de argumento **nunca mata o processo**: devolve erro. Só a falha de CPU (instrução
ilegal, falta sem região) mata, pelo [canal de eventos](#falhas-de-usuário).

### As chamadas

18 chamadas e um `nop` de medida. A pesquisa listava "11" contando grupos (`handle_*`,
`channel_*`, `vmar_*`); contando cada chamada, e com o `random_get`, são estas.

| # (proposta) | Nome | Argumentos | Valor de retorno | Direito exigido |
| --- | --- | --- | --- | --- |
| `0x00` | `nop` | — | — | Nenhum. Só para a medida da syscall nula |
| `0x01` | `process_exit` | `code: i64` | não volta | Nenhum |
| `0x02` | `thread_exit` | — | não volta | Nenhum |
| `0x03` | `debug_write` | `console`, `ptr`, `len` (**4a: sem `console`**) | bytes escritos | `console`: `WRITE` |
| `0x04` | `clock_get_monotonic` | — | ns (`u64`), ver [contrato](#contrato-numérico) | Nenhum |
| `0x05` | `random_get` | `ptr`, `len` (até 256) | — | Nenhum |
| `0x10` | `handle_close` | `h` | — | Nenhum |
| `0x11` | `handle_duplicate` | `h`, `rights` | handle novo | `h`: `DUPLICATE`; `rights ⊆ h.rights` |
| `0x12` | `handle_replace` | `h`, `rights` | handle novo | **Nenhum** (verificado no Zircon); `rights ⊆ h.rights` |
| `0x13` | `object_wait_one` | `h`, `signals`, `deadline_ns` | sinais observados | `h`: `WAIT` |
| `0x20` | `vmo_create` | `size`, `flags` (0) | handle do VMO | Nenhum (ver acima) |
| `0x21` | `vmar_map` | `vmar`, `vmo`, `vmo_off`, `len`, `flags`, `vmar_off` | endereço mapeado | `vmar`: `MAP`; `vmo`: `MAP` e cada permissão pedida (`READ`/`WRITE`/`EXECUTE`), todas também no teto do `vmar` |
| `0x22` | `vmar_unmap` | `vmar`, `addr`, `len` | — | `vmar`: `MAP` |
| `0x30` | `process_create` | `parent`, `name_ptr`, `name_len`, `bootstrap`, `events` | processo e VMAR raiz (empacotados) | `parent`: `MANAGE_PROCESS`; `bootstrap`: `TRANSFER` (move); `events`: `WRITE` e `TRANSFER` (move; 0 = nenhum) |
| `0x31` | `thread_create` | `process`, `entry`, `sp`, `arg0`, `arg1` | handle da thread | `process`: `MANAGE_THREAD`. Cria **e inicia**; a primeira thread inicia o processo |
| `0x32` | `process_kill` | `process` | — | `process`: `DESTROY` |
| `0x40` | `channel_create` | `flags` (0) | dois handles (empacotados) | Nenhum |
| `0x41` | `channel_write` | `ch`, `bytes`, `nbytes`, `handles`, `nhandles` | — | `ch`: `WRITE`; cada handle: `TRANSFER`. [Consome o array](#operações-sobre-handle) |
| `0x42` | `channel_read` | `ch`, `bytes`, `bytes_cap`, `handles`, `handles_cap`, `actual*` | — (tamanhos em `actual`) | `ch`: `READ`. Só retira a mensagem **depois** da cópia ([ordem](#operações-sobre-handle)) |

- **`thread_create` e o carregador validam `USER_MIN <= entry < USER_TOP`, `sp <= USER_TOP` e `sp` alinhado a 16** antes de qualquer outra coisa; fora disso, `InvalidArgs`. Teste no host com `0xFFFF_8000_0000_0000`, `0x0000_8000_0000_0000` e `USER_TOP` (o `RIP` que o `IRETQ` restaura vem daqui: [exceções do anel 3](#exceções-vindas-do-anel-3)).
- **`vmar_map`:** `flags` leva `READ`/`WRITE`/`EXECUTE` e, opcionalmente, `FIXED` (`vmar_off` exato; **recusa** sobrepor mapeamento existente, o equivalente do `MAP_FIXED_NOREPLACE` do Linux, **verificado** em [mmap(2)](https://man7.org/linux/man-pages/man2/mmap.2.html); o `MAP_FIXED` do Linux apaga o que havia, e o RustOS não o oferece). Sem `FIXED` o kernel escolhe o endereço, com o [ASLR](#aslr-quantos-bits), e `vmar_off` tem de ser 0. W+X é `InvalidArgs`. Intervalo na faixa nula ou acima de `USER_TOP` é `OutOfRange`.
- **`debug_write`:** os bytes passam por filtro antes do console (descarta ESC, controle exceto `\n` e `\t`, e `0x7F`: sem injeção de sequência de terminal) e a chamada escreve no máximo 512 bytes (o lock do console mascara IRQ; [DEBT-0014](../tech-debt/0014-x86-sem-salvaguardas-de-metal.md)).
- **`process_create`** exige um handle de processo-pai com `MANAGE_PROCESS`. O kernel dá ao `init` um handle de **si mesmo** com esse direito; o `init` decide a quem duplicar. A alternativa, um objeto `Job` como o do Zircon, fica para os grupos de recursos (fase 9).
- **Processo sem `events`:** morre do mesmo jeito; o pai só vê `TERMINATED`.

### Contrato numérico

**Sinais** de `object_wait_one` (nível, não borda; `signals == 0` é `InvalidArgs`):

| Sinal | Valor | Vale para |
| --- | --- | --- |
| `READABLE` | `1 << 0` | Canal com mensagem na fila |
| `WRITABLE` | `1 << 1` | Canal com espaço na fila da outra ponta |
| `PEER_CLOSED` | `1 << 2` | Canal: a outra ponta morreu |
| `TERMINATED` | `1 << 3` | Process e Thread que terminaram |

**Numeração diferente de propósito:** as flags de `vmar_map` (`READ` `1 << 0`, `WRITE` `1 << 1`, `EXECUTE` `1 << 2`) e os bits de direito (`READ` bit 2, `WRITE` bit 3, `EXECUTE` bit 4) **não coincidem**. Flag é argumento de uma chamada; direito é atributo de um handle. Misturá-los (usar o direito como flag, ou o contrário) seria erro silencioso; a conversão é uma tabela explícita, com teste. **Flags:** `vmar_map`: `READ` `1 << 0`, `WRITE` `1 << 1`, `EXECUTE` `1 << 2`, `FIXED` `1 << 3`; o resto é reservado e dá `InvalidArgs`. `vmo_create` e `channel_create`: `flags` tem de ser 0. **Direitos:** o bit `n` da [tabela](#direitos-iniciais-decisão-3) vale `1 << n`.

**Direitos iniciais de cada handle que o kernel cria:**

| Chamada | Objeto | Direitos |
| --- | --- | --- |
| `vmo_create` | VMO | `DUPLICATE`, `TRANSFER`, `READ`, `WRITE`, `EXECUTE`, `MAP`, `INSPECT` (o W^X é do `vmar_map`, não do handle) |
| `channel_create` | As duas pontas | `DUPLICATE`, `TRANSFER`, `READ`, `WRITE`, `WAIT` |
| `process_create` | Process | `DUPLICATE`, `TRANSFER`, `WAIT`, `INSPECT`, `DESTROY`, `MANAGE_PROCESS`, `MANAGE_THREAD` |
| `process_create` | VMAR raiz | `DUPLICATE`, `TRANSFER`, `READ`, `WRITE`, `EXECUTE`, `MAP` |
| `thread_create` | Thread | `DUPLICATE`, `TRANSFER`, `WAIT`, `INSPECT` |
| Boot do `init` | Process dele | Os mesmos de `process_create` |
| Boot do `init` | Console | `DUPLICATE`, `TRANSFER`, `WRITE` |
| Boot do `init` | Initramfs (VMO físico) | `DUPLICATE`, `TRANSFER`, `READ`, `MAP` (sem `EXECUTE`, sem `WRITE`) |

**Handle:** `u32`, `0` inválido, geração de 16 bits com aposentadoria do slot, teto de 4 096 em tabela por processo e **orçamento de cerca de 2^32 handles por processo na vida** ([tabela de handles](#tabela-de-handles); a largura é pergunta da decisão 2).

**Limites do canal.** Comparados com o Zircon (**verificado**: 65 536 bytes e 64 handles por mensagem; a profundidade da fila **NÃO VERIFICADO**):

| | RustOS (proposta) | Zircon |
| --- | --- | --- |
| Bytes por mensagem | 1 024 | 65 536 |
| Handles por mensagem | 8 | 64 |
| Mensagens na fila de cada ponta | 16 | não lido |
| Pior caso por ponta | 16 KiB de bytes mais 128 handles | — |

São menores de propósito (a meta de memória): a mensagem de boot cabe, e dado maior vai por
VMO. O custo é que um protocolo que precise de mensagem grande tem de usar VMO.

**`clock_get_monotonic`:** nanossegundos desde o boot do kernel (o relógio do [SDD 10](10_interrupcoes.md): TSC calibrado no x86_64, `CNTVCT_EL0` no aarch64), monotônico, `u64`, sem suspensão a descontar. Resolução: a do contador. **`deadline_ns`** é absoluto nesse relógio; `deadline <= agora` consulta sem bloquear (devolve os sinais se já valem, senão `TimedOut`); `u64::MAX` espera sem fim. Não há acordar falso: a chamada reavalia os sinais antes de voltar.

**`ProcessEvent`** (`#[repr(C)]`, 56 bytes, uma mensagem no canal de eventos; sem handles):

| Offset | Campo | Tipo | Valor |
| --- | --- | --- | --- |
| 0 | `kind` | `u32` | `1` Exit, `2` Fault |
| 4 | `fault_kind` | `u32` | `0` (Exit); Fault: `1` falta de página, `2` instrução ilegal, `3` acesso desalinhado, `4` aritmética, `5` breakpoint, `6` outro |
| 8 | `access` | `u32` | Falta de página: bit 0 escrita, bit 1 execução, bit 2 violação de proteção (senão, não presente) |
| 12 | `reserved` | `u32` | 0 |
| 16 | `code` | `i64` | Exit: o argumento de `process_exit`; Fault: 0 |
| 24 | `pc`, 32 `sp`, 40 `addr` | `u64` | Fault, do usuário (**nunca** endereço do kernel) |
| 48 | `isa_code` | `u64` | O `code` cru da ISA (vetor e erro no x86_64, `ESR_EL1` no aarch64) |

**Mensagem de boot** (`#[repr(C)]`; cabe em 1 KiB): cabeçalho `{ magic: u32 = 0x544F_4F42, version: u32 = 1, count: u32, reserved: u32 }` seguido de `count` valores `u32` de `kind`; o **i-ésimo handle do array de handles da mensagem** corresponde ao i-ésimo `kind`. `kind`: `1` Process (si mesmo), `2` VMAR raiz, `3` Console, `4` Initramfs. Os mesmos tipos servem a um pai qualquer.

### O que a ABI deixa de fora de propósito

`vmo_read`/`vmo_write` (mapeia e copia), `vmar_protect` (o carregador mapeia cada
segmento com a permissão final), sub-VMARs (`vmar_allocate`), `object_wait_many`, portas,
eventos, `futex`, `thread_suspend`, `object_get_info`, `Job`. Cada uma volta quando um
programa precisar dela, com o caso de uso no PR.

### Estabilidade

A ABI nativa é **experimental até o fim da fase 5**: pode mudar sem compatibilidade. A
partir daí o que existir é contrato. Está na [decisão 2](18_fase_4_decisoes_pedidas.md#decisão-2--abi-nativa-lista-números-nomes-e-contrato).

### Variante enxuta (16 chamadas)

Sem `thread_create`, `thread_exit` e `process_kill`, e com `process_start` no lugar (uma
thread por processo): 16 chamadas, ainda acima de "11 a 15". Custo: sem multithread
nativo até a fase 5, e o pai só mata o filho pelo canal ou esperando.

## Executável e carregador

### Formato: ELF64 estático (proposta, [decisão 1](18_fase_4_decisoes_pedidas.md#decisão-1--formato-do-executável-nativo))

| Item | Valor | Fonte |
| --- | --- | --- |
| `e_machine` | `62` (x86_64), `183` (aarch64) | **Verificado**: `include/uapi/linux/elf-em.h` |
| Classe e ordem | 64 bits, little-endian; valores de `EI_CLASS`/`EI_DATA` | **NÃO VERIFICADO** (a gABI não abriu na pesquisa: HTTP 404); conferir em `<elf.h>` |
| Tipo | `ET_EXEC`. `ET_DYN` só com a [decisão 5](18_fase_4_decisoes_pedidas.md#decisão-5--pie-de-usuário-agora-ou-depois) | — |
| Seções | O kernel **nunca** lê section headers | **Proposta** |

### Validação, regra por regra

O molde é `fs/binfmt_elf.c` do Linux (**verificado**, [fonte](https://github.com/torvalds/linux/blob/master/fs/binfmt_elf.c)).
A validação mora em `core/elf` (`forbid(unsafe_code)`, `&[u8]` para uma `Image`; depende só do `hal`, de onde vêm `USER_MIN`, `USER_TOP` e `PAGE_SIZE`),
testada no host com ELF sintético e com os casos inválidos (guidelines 09).

| Regra | De onde |
| --- | --- |
| `e_ident` começa com `ELFMAG` | Linux (verificado) |
| `e_type` é `ET_EXEC` (ou `ET_DYN`, se a [decisão 5](18_fase_4_decisoes_pedidas.md#decisão-5--pie-de-usuário-agora-ou-depois) permitir) | Linux (verificado) |
| Máquina e classe batem com a ISA do kernel (`elf_check_arch`) | Linux (verificado; `EI_CLASS`/`EI_DATA` estão dentro dela, não visíveis no trecho lido) |
| `e_phentsize == size_of::<Phdr>()` | Linux (verificado) |
| `e_phoff + e_phnum * e_phentsize` sem estouro e **dentro do arquivo** (a tabela de program headers inteira) | **Proposta** (própria) |
| Tamanho total dos program headers entre 1 e 65 536 bytes | Linux (verificado) |
| `p_filesz <= p_memsz` | Linux (verificado) |
| `p_align` é potência de dois | Linux (verificado) |
| `PT_INTERP` **recusado** (o Linux o aceita, limitado e terminado em NUL; a fase 4 não tem interpretador) | Linux (verificado) e decisão do RustOS |
| `p_offset + p_filesz` sem estouro e dentro do arquivo | Proposta |
| `PT_LOAD` com `p_memsz == 0` **recusado** (não mapeia nada; é erro do linker) | Proposta |
| `p_vaddr + p_memsz` sem estouro, **e os limites conferidos depois de arredondar à página**: início arredondado para baixo e fim arredondado para cima, ambos dentro de `[USER_MIN, USER_TOP]`, sem estouro no arredondamento | Proposta |
| `p_vaddr` e `p_offset` congruentes módulo o tamanho da página | Proposta; a regra é do ELF, **NÃO VERIFICADO** na gABI |
| **Nenhum `PT_LOAD` com W e X** (W^X por segmento) | Proposta; é o que o mapeador já exige |
| `PT_LOAD` ordenados por `p_vaddr`, sem sobreposição (em granularidade de página) | Proposta |
| `e_entry` dentro de um `PT_LOAD` executável | Proposta |
| Todo `PT_LOAD` com `R` | Proposta |
| No máximo 16 `PT_LOAD` e `p_memsz` total até um teto (ex.: 64 MiB) | Proposta |
| `PT_TLS`, `PT_DYNAMIC` recusados na fase 4 (sem TLS, sem ligação dinâmica) | Proposta |
| `PT_GNU_STACK`, `PT_GNU_RELRO`, `PT_NOTE` e tipos desconhecidos ignorados; a pilha nunca é executável | Proposta |

### Auxv mínimo

Descrições **verificadas** em [getauxval(3)](https://man7.org/linux/man-pages/man3/getauxval.3.html); os valores numéricos, **verificados** em
`include/uapi/linux/auxvec.h` do Linux.

| Tag | Valor | O quê | Fase 4 |
| --- | --- | --- | --- |
| `AT_NULL` | 0 | Fim do vetor | sim |
| `AT_PHDR` | 3 | Endereço dos program headers na memória | sim (só se a tabela cair dentro de um `PT_LOAD`) |
| `AT_PHENT` | 4 | Tamanho de cada entrada | sim |
| `AT_PHNUM` | 5 | Número de entradas | sim |
| `AT_PAGESZ` | 6 | Tamanho da página | sim (4096) |
| `AT_ENTRY` | 9 | Entrada do executável | sim |
| `AT_HWCAP` | 16 | Máscara da arquitetura | **0**; o formato é a decisão futura da fase 5 ([SDD 08](08_roadmap.md#decisões-futuras-casos-do-agentsmd)) |
| `AT_RANDOM` | 25 | Endereço de 16 bytes aleatórios | sim, da `random::` do kernel |
| `AT_HWCAP2` | 26 | Extensão | **0** |
| `AT_EXECFN` | 31 | Caminho usado | não (não há caminho) |

O Linux x86 preenche `AT_HWCAP` com `CPUID.1:EDX` (**verificado**, `elf.h`, pesquisa
`fpu-simd.md`); o RustOS não o copia agora. O programa nativo lê `CPUID` e `XGETBV`
direto ([SIMD de usuário](#simd-de-usuário)).

**Pilha inicial** no formato System V (`argc`, `argv`, `NULL`, `envp`, `NULL`, `auxv`),
com `rsp` alinhado a 16: **NÃO VERIFICADO**, conferir no psABI de x86-64 e na convenção
de processo do aarch64 antes de codar. A entrada do programa recebe também o handle do
canal de boot em `rdi`/`x0` (**Proposta**, no espírito do `zx_process_start`, cuja página
não foi lida).

### Como o `init` e os outros programas chegam à memória

| Quem | Como |
| --- | --- |
| `init` | O **kernel** o carrega: `core/elf` valida; para cada `PT_LOAD` aloca frames, copia os bytes do arquivo, deixa o resto zerado (`.bss`, inclusive a cauda da última página de arquivo) e os mapeia, com a permissão do segmento, **direto no espaço do processo** pela árvore de regiões de `core/mm` (4b: **sem objeto VMO nem VMAR e sem handle**; o VMO e o VMAR raiz como objetos só existem a partir da 4c, e então o `init` passa a receber o handle do VMAR raiz). Cópia, não mapa em lugar: os dados do `newc` só alinham a 4 bytes |
| Outros programas | Quem os cria. O `init` lê o ELF do initramfs (que recebeu como VMO), cria o processo, e preenche e mapeia os VMOs com as chamadas da ABI; o `user/rt` usa o mesmo `core/elf` |

Só o `init` está no caminho de confiança do kernel (initramfs do bootloader; boot
verificado é da fase 12). O carregador do usuário **não** é fronteira de segurança: o que
o kernel garante (W^X, faixas, direitos) vale na `vmar_map`, qualquer que seja o parser.

### O que muda no linker e no alvo

[SDD 05](05_build_e_execucao.md): os dois alvos são `x86_64-unknown-none` (code model
`kernel`) e `aarch64-unknown-none-softfloat`, Tier 2, sem `build-std`. Os programas de
usuário usam os mesmos alvos, com flags diferentes: `-C code-model=small` (x86_64, o do
`kernel` não serve para endereço baixo: **NÃO VERIFICADO** que o linker aceite, testar na
4b), `-C relocation-model=static` (PIE só com a [decisão 5](18_fase_4_decisoes_pedidas.md#decisão-5--pie-de-usuário-agora-ou-depois)), linker script próprio
(`user/link/<isa>.ld`, base `0x40_0000`, um segmento por permissão, `-z max-page-size=0x1000`)
e `--target-dir` separado. O `.cargo/config.toml` de hoje põe `relocation-model=pie` nos
dois alvos: o xtask passa as flags de usuário por `CARGO_ENCODED_RUSTFLAGS`, que (**NÃO
VERIFICADO** na doc do Cargo) substitui as do `config.toml`.

## Init

O `init` é o primeiro processo e o único que o kernel lança sozinho.

### Origem ([decisão 4](18_fase_4_decisoes_pedidas.md#decisão-4--de-onde-vem-o-init))

**Verificado** (cpio `newc`, [early-userspace buffer format](https://docs.kernel.org/driver-api/early-userspace/buffer-format.html)):

| Item | Valor |
| --- | --- |
| Magic | `"070701"` (ou `"070702"`, com checksum) |
| Campos | 13 campos de 8 caracteres hexadecimais ASCII depois do magic: `c_ino`, `c_mode`, `c_uid`, `c_gid`, `c_nlink`, `c_mtime`, `c_filesize`, `c_maj`, `c_min`, `c_rmaj`, `c_rmin`, `c_namesize`, `c_chksum` (zero no `070701`). Cabeçalho de 110 bytes (6 + 13 × 8) |
| Gramática | `cpio_file := ALGN(4) + cpio_header + filename + "\0" + ALGN(4) + data`, com `ALGN(n)` = preenchimento com bytes nulos até múltiplo de `n`. **O nome não alinha sozinho**: o registro (cabeçalho mais nome mais o NUL) e os dados alinham a 4, e não há preenchimento entre o cabeçalho e o nome |
| `c_namesize` | "Length of filename, including final 0"; **pode contar mais de um NUL final** (até `PATH_MAX`): o leitor lê o nome até o primeiro NUL, mas **avança por `c_namesize`** |
| `c_mode` | É o `st_mode` do `stat(2)`: tipo e permissões. [inode(7)](https://man7.org/linux/man-pages/man7/inode.7.html): `S_IFMT` `0170000`, `S_IFREG` `0100000`, `S_IFDIR` `0040000`, `S_IFLNK` `0120000` |
| Hard link | Entrada com `c_nlink > 1`: o Linux a liga ao `(c_maj, c_min, c_ino)` já visto e aceita os dados em qualquer ocorrência |
| Fim | `TRAILER!!!`, com `c_filesize` zero. No Linux ele é opcional, "mas não é ignorado"; o `core/cpio` o **exige** (regra própria, mais estrita) |

O leitor (`core/cpio`) é um iterador sem alocação: confere o magic, cada dígito
hexadecimal, `c_namesize` e `c_filesize` contra o fim do buffer (sem estouro), o NUL do
nome e a presença do `TRAILER!!!`, e usa o alinhamento da gramática acima. Aceita só
arquivo regular (`c_mode & S_IFMT == S_IFREG`) com `c_nlink == 1`; rejeita diretório que
tenha dado, link simbólico, dispositivo e hard link. O `init` é o arquivo de nome `init`.

**Limine** (lido na cópia local do `PROTOCOL.md` v8.x; o projeto usa a revisão base 6 e
o Limine 12.10.0, então **conferir contra o `limine-protocol` da versão em uso**):

| Item | Valor |
| --- | --- |
| Pedido | `LIMINE_MODULE_REQUEST`, id `{ COMMON_MAGIC, 0x3e7e279702be32af, 0xca1c4f3bd1280cee }` |
| Resposta | `module_count` e `modules`: ponteiros para `struct limine_file` |
| `limine_file` | `revision`, `address`, `size`, `path`, `cmdline`, `media_type`, e identificadores de partição |
| Memória | O módulo carregado é marcado `Executable/Modules`, não `Usable` |
| Chave do módulo no `limine.conf` | **NÃO VERIFICADO** (a `CONFIG.md` não foi lida) |

Como o módulo cai em `KernelAndModules`, ele **fica fora do mapa direto** ([SDD 13](13_memoria_virtual.md)). O desenho:

| Passo | O quê |
| --- | --- |
| `boot/limine` | Novo pedido de módulo; `hal::BootInfo` ganha `modules()` (porta nova no `hal`, decisão do agente); o `BootRecord` copia `(físico, tamanho)` |
| `kernel/src/vm.rs` | Mapeia a faixa do módulo só leitura e NX, numa janela (como a imagem tem as dela); o W^X confere |
| Initramfs como VMO | **A partir da 4c** (na 4b o kernel só mapeia a faixa e lê o `init` dela): o arquivo vira um **VMO físico** (frames que já existem, sem zerar) com `READ`, `MAP`, `DUPLICATE`, `TRANSFER`, e vai ao `init` pelo canal de boot |
| Devolver ao buddy | Quando o último handle do VMO fechar, devolver os frames do módulo. Que o protocolo permita reusar a memória `Executable/Modules` que não é da imagem **NÃO VERIFICADO**; se não permitir, o initramfs fica residente |

### O canal de boot

Mensagem única, escrita pelo kernel antes de iniciar o `init`, com um par
`(tipo, handle)` por item: o Process do próprio `init` (todos os direitos de processo),
o VMAR raiz, o console de depuração (`WRITE`, `DUPLICATE`, `TRANSFER`) e o VMO do
initramfs. O formato é um `#[repr(C)]` em `hal::abi`, igual no kernel e no `user/rt`. O
mesmo mecanismo serve a qualquer pai: ele escreve a mensagem e passa a ponta do canal em
`process_create(.., bootstrap, ..)`.

## Falhas de usuário

Hoje toda exceção não recuperável para a CPU ([DEBT-0012](../tech-debt/0012-excecao-de-usuario-so-para-a-cpu.md)). O desenho muda o destino de quem veio de usuário.

| Distinção | Como |
| --- | --- |
| Origem | x86_64: `CS` salvo com `RPL = 3` (o mesmo teste do `swapgs`). aarch64: grupo do vetor (EL inferior, 8 a 11). `hal::Fault` ganha `from_user: bool` e o endereço da falta |
| Regra | **Falha de usuário mata o processo; o kernel segue.** Falha do kernel continua parando, com relatório |

| Evento vindo de usuário | O que acontece |
| --- | --- |
| Falta de página dentro de um VMAR, com permissão que serve | Resolve (página zero ou frame novo) e volta; o tratador **nunca bloqueia** (sem swap nem arquivo mapeado), roda mascarado como o resto |
| Falta de página fora de região, na guarda, ou violação de permissão | Falha: processo morre |
| `#UD`, `#GP`, `#DE`, `#AC`, `#XM`/`#MF`, `int3` (x86_64) | Falha |
| `EC` `0x00`, `0x18`, `0x19`, `0x1D`, `0x20`/`0x24` sem resolução (aarch64) | Falha (SVE e SME trapeiam porque a fase 4 não os liga: [DEBT-0035](../tech-debt/0035-sve-sme-e-amx-sem-estado-por-tarefa.md)) |
| Syscall com número inválido | `NotSupported`; **não** mata |
| IRQ | Normal, depois volta ao usuário (a preempção passa por `Handlers.preempt`) |
| `NMI`, `#DF`, `#MC`, `SError` | Continuam fatais, com relatório, mesmo vindo de usuário |

**A morte do processo, em ordem** (cada passo tem uma razão; trocar a ordem quebra):

| # | Passo | Por quê |
| --- | --- | --- |
| 1 | Marca o processo como "em morte"; novas chamadas que o miram dão `BadState` | Fecha a corrida com `thread_create` e `process_kill` de outro processo |
| 2 | Cada thread do processo sai das filas em que está: **a thread em `object_wait_one` é retirada da lista de espera de cada objeto**, e o escalonador a remove (`remove`) | Senão o objeto guarda o `TaskId` de uma thread morta e o acorda depois |
| 3 | **Troca o `CR3`/`TTBR0` para a raiz do kernel (x86_64) ou a raiz vazia (aarch64)** antes de liberar qualquer tabela | A tabela liberada não pode estar em uso pela CPU; vale também quando a thread que morre é a que está rodando |
| 4 | Fecha todos os handles (`on_zero_handles` nos objetos; canais ganham `PEER_CLOSED`) | Os objetos soltos destroem em cascata |
| 5 | Desmapeia e devolve os frames, zerados ao liberar ([SDD 09](09_seguranca.md)); depois as tabelas de página e o ASID/PCID | Ver o passo 3 |
| 6 | Levanta `TERMINATED` no Process e escreve o evento no canal de eventos | Quem espera já enxerga o estado final |
| 7 | **A pilha de kernel da thread que sai só é liberada depois do `switch`**, por quem entra (ou por uma lista de recolhimento que a ociosa esvazia) | A thread não pode liberar a pilha em que ela mesma roda; o slot `Exited` já mantém a pilha ([SDD 15](15_tarefas_e_preempcao.md)), e o endereço só volta à lista de livres depois |

### Canal de eventos ([decisão 6](18_fase_4_decisoes_pedidas.md#decisão-6--falha-de-usuário-canal-de-eventos-ou-sinal-síncrono))

O pai passa uma ponta de canal em `process_create(.., events)`. Quando o processo termina,
o kernel escreve **uma** mensagem de `ProcessEvent` nela:

| Campo | Para `Exit` | Para `Fault` |
| --- | --- | --- |
| `kind` | `Exit` | `Fault` |
| `code` | o argumento de `process_exit` | — |
| `fault_kind` | — | falta de página, instrução ilegal, acesso desalinhado, aritmética, breakpoint, outro |
| `pc`, `sp`, `addr` | — | do usuário |
| `isa_code` | — | o `code` cru da ISA (vetor e erro; `ESR_EL1`) |

Regras: **só endereço de usuário** na mensagem (o endereço do kernel não sai: [DEBT-0022](../tech-debt/0022-endereco-do-kernel-no-log-de-boot.md)); sem canal, o processo morre do mesmo
jeito e o pai só vê `TERMINATED`. Na fase 4 o fim é sempre a morte: **não há retomada**
de exceção. O Linux usa sinais síncronos (SIGSEGV, SIGBUS, SIGILL, SIGFPE) entregues à
thread, com ação padrão `Core` (**verificado**, [signal(7)](https://man7.org/linux/man-pages/man7/signal.7.html)); a ABI nativa não os copia.
O adaptador Linux da fase 5 traduz o `Fault` em sinal.

## SIMD de usuário

| Ponto | Desenho |
| --- | --- |
| `simd` | `true` para **toda** thread de usuário. É a primeira das duas saídas da [DEBT-0043](../tech-debt/0043-simd-sem-trap-de-uso-indevido.md): o ABI do x86_64 exige SSE, e o salvamento eager é a escolha já feita (LazyFP; [SDD 09](09_seguranca.md)). A saída por trap (`CR0.TS`/#NM) fica descartada, e o #NM só voltaria com medida que a justifique |
| Custo por thread | 832 bytes de área no x86_64 da QEMU e 528 no aarch64 ([SDD 15](15_tarefas_e_preempcao.md)); um `XSAVEOPT`/`XRSTOR` por troca (medido no [SDD 07](07_desempenho_e_recursos.md)) |
| Estado inicial | O de [SDD 15](15_tarefas_e_preempcao.md): `FCW` `0x037F`, `MXCSR` `0x1F80`, `XSTATE_BV` zero; aarch64 com V0–V31, `FPSR` e `FPCR` zerados |
| `CPUID` em anel 3 | Não é trapeado por padrão (pesquisa `fpu-simd.md`; SDM não lido) |
| `XGETBV` em anel 3 | Funciona e devolve o `XCR0` que o kernel ligou (x87, SSE, AVX, AVX-512 se a CPU tem, PKRU); a doc do kernel diz para "consult CPUID and use XGETBV to evaluate which features have been enabled by the kernel" (pesquisa `fpu-simd.md`) |
| `AT_HWCAP` | 0 na fase 4 (decisão futura da fase 5) |
| AMX | **Fora** do `XCR0` ([SDD 08](08_roadmap.md)); instrução de AMX dá `#UD` e o processo morre. O pedido de AMX só é preciso com sinais (sigframe), que a fase 4 não tem |
| SVE e SME | `CPACR_EL1.ZEN`/`SMEN` seguem zero: acesso trapeia (`EC` `0x19`, `0x1D`) e o processo morre. [DEBT-0035](../tech-debt/0035-sve-sme-e-amx-sem-estado-por-tarefa.md) segue aberta, com o gatilho reescrito para o primeiro programa que use SVE, SME ou AMX, ou o primeiro sinal Linux |
| `rdtsc` | `CR4.TSD = 0` já é o padrão. A opção por processo (`PR_SET_TSC`) **continua decisão futura**; a fase 4 não a oferece |
| Teste | `user/` roda duas threads em processos diferentes, cada uma põe um valor em `xmm0`/`v0` e o confere em laço. O `asm!` está num crate de `arch/` ([onde mora](#onde-cada-peça-mora-no-hexágono)) |

## Onde cada peça mora no hexágono

### Crates novos

**Crate novo em `core/` e pasta nova de primeiro nível são o caso 1**; a regra 5 do guidelines
03 que muda é o caso 3. As decisões [7](18_fase_4_decisoes_pedidas.md#decisão-7--crates-novos-em-core) e [8](18_fase_4_decisoes_pedidas.md#decisão-8--pasta-user-e-a-regra-5-do-guidelines-03) pedem a aprovação.

| Crate | Pasta | O que faz | `unsafe` | Depende de |
| --- | --- | --- | --- | --- |
| `rustos-elf` **(novo)** | `core/elf` | Validação e leitura de ELF64: `&[u8]` para `Image { entry, loads, phdr }` | proibido | `rustos-hal` (`abi`: `USER_MIN`, `USER_TOP`, `PAGE_SIZE`) |
| `rustos-cpio` **(novo)** | `core/cpio` | Leitor `newc`, iterador sem alocação | proibido | nada |
| `rustos-object` **(novo)** | `core/object` | `Rights`, `HandleTable<T>`, regras de duplicar, substituir e transferir, contagem de handles, fila do canal, sinais | proibido | `rustos-hal` |
| ABI (números, `Status`, direitos, constantes de layout, `ProcessEvent`, mensagem de boot) | `core/hal/src/abi.rs` (**módulo, não crate**) | O contrato com o programa de usuário, a "primeira porta de entrada" do [SDD 02](02_arquitetura.md) | proibido | nada |
| `rustos-user-rt` **(novo)** | `user/rt` | Runtime dos programas: ponto de entrada em Rust, `panic`, alocador mínimo, leitor de auxv, carregador de ELF de usuário | **mínimo, com `// SAFETY:`**: `GlobalAlloc`, a leitura do auxv e `#[unsafe(no_mangle)]` o exigem; nenhum `asm!` | `hal` (a ABI), `elf`, `cpio`, `arch-<isa>-user` |
| `rustos-user-init` e programas de teste **(novos)** | `user/init`, `user/tests/*` | O `init`, o teste de falha, o de SIMD, o de syscall nula | proibido (`forbid(unsafe_code)`; o `unsafe` fica no `user/rt`) | `user-rt` |
| `rustos-arch-x86_64-user`, `rustos-arch-aarch64-user` **(novos)** | `arch/x86_64-user`, `arch/aarch64-user` | **Todo o `asm!` do lado do usuário:** `_start` (alinha o `rsp`, zera o frame pointer e chama o `main` do `user/rt`), os stubs de `syscall`/`svc`, `rdtsc`/`cntvct` e `xmm0`/`v0` do teste | permitido | `rustos-hal` |

O `_start` precisa de `asm!` (o alinhamento do `rsp`, e nenhuma função Rust nasce sem
prólogo); como o `asm!` só mora em `arch/`, ele vai para `arch/<isa>-user`. O `user/rt`
não tem `asm!`, mas **tem `unsafe` mínimo, com `// SAFETY:`**: as regras do
[guidelines 04](../guidelines/04_unsafe_e_asm.md) já permitem (o `forbid(unsafe_code)` é só de
`core/` e `drivers/`), e o `check-layers` da 4a o confere (sem `asm!` em `user/`). Os adaptadores `arch/<isa>-user` cabem nas regras 4 e 8 do guidelines
03; o que muda é a pasta `user/` e a regra 5. O **diff está feito** em
[`guidelines/03`](../guidelines/03_camadas_e_dependencias.md).

A ABI vai num módulo do `hal`, e não num crate `core/abi`, porque um crate de `core/` que
outros crates de `core/` e o `user/` importassem quebraria a regra 2 (caso 3). Custo: o
`user/` compila o `hal` inteiro (só tipos, `no_std`, sem `unsafe`; o que não é usado some
na ligação).

Sem crate novo (decisão do agente, [`AGENTS.md`](../../AGENTS.md)):

| Peça | Onde |
| --- | --- |
| Árvore de regiões do VMAR, alocação com ASLR, bookkeeping de mapeamentos | `core/mm/src/vmar.rs` (módulo novo) |
| Gerador de ASID/PCID | `core/mm/src/asid.rs` |
| `UserRange` | `core/mm/src/user.rs` |
| Entrada de syscall (`asm!`), `enter_user`, `usercopy`, área por CPU | `arch/<isa>/src/{syscall,user,usercopy,percpu}.rs`; a parte pura (a checagem do `SYSRET`, o valor do `STAR`, a decodificação do `EC` de SVC) em módulo próprio, testada no host |
| `PageFlags.user` e as codificações | `core/hal/src/paging.rs`, `arch/*/src/{pte,descriptor}.rs` |
| `Fault.from_user` e `addr` | `core/hal/src/interrupt.rs` |
| `BootInfo::modules()` | `core/hal/src/boot.rs`, `boot/limine` |
| Process, Thread, Vmo, VMAR, Channel, despacho de syscall, carregador do `init`, espaço de usuário | `kernel/src/{process,object,syscall,loader,uvm}.rs` |
| Montagem do initramfs, build dos programas | `xtask/src/{user,initramfs}.rs` |

### O que muda no kernel

| Ponto | Mudança |
| --- | --- |
| `MAX_TASKS` e `Composite<4, 16>` | Sobem (cada tarefa custa cerca de 250 bytes em `.data`; [SDD 15](15_tarefas_e_preempcao.md)); a capacidade vira configurável ou cresce do heap. O número entra no PR |
| Pilha de kernel por thread de usuário | 8 páginas como as de hoje, **até a medida de pico** ([DEBT-0037](../tech-debt/0037-pilha-de-tarefa-sem-medida-de-pico.md)); a janela de pilhas passa a reciclar endereço (lista livre), senão laço de criar e matar esgota a janela de 1 GiB |
| Troca de contexto | Se o processo muda, troca `CR3`/`TTBR0` (com PCID/ASID) e escreve `kernel_rsp`/`TSS.rsp[0]` |
| Contas de tempo | O tempo de CPU por tarefa já existe; processo soma as threads |
| `ps` | Uma coluna de processo; a de SIMD já existe |

### Como o xtask compila e empacota, sem crate externo

| Passo | Comando (proposta) | O quê |
| --- | --- | --- |
| 1 | `cargo xtask user --arch A` | Compila cada crate de `user/` com as flags de usuário e `--target-dir target/user/<isa>` |
| 2 | `cargo xtask initramfs --arch A` | Escreve `target/initramfs/<isa>.cpio` em `newc`, com um escritor em Rust puro dentro do `xtask` (cerca de 60 linhas; o `xtask` não depende de crate do workspace, regra 6, então não reaproveita o `core/cpio`) e testado contra bytes de ouro; sem o binário `cpio` do host |
| 3 | `cargo xtask image` | Copia o cpio para `boot/initramfs.cpio` na ESP e acrescenta o módulo ao `limine.conf` |
| 4 | `make smoke` | Exige as linhas novas ([abaixo](#linhas-que-o-smoke-passa-a-exigir)) |

A ordem de build é usuário, initramfs, imagem; o kernel **não** depende dos programas de
usuário, então mexer no `init` não recompila o kernel.

### Linhas que o smoke passa a exigir

Proposta; o texto final entra com o código. Em inglês, como toda saída serial.

| Linha | Fatia |
| --- | --- |
| `[rustos] user: entered ring3/el0` | 4a |
| `[rustos] user: fault contained (pid N)` | 4a |
| `[rustos] user: init loaded (N segments)` | 4b |
| `[rustos] mitigations: kpti=… ` | 4e |
| `[rustos] user: self-test ok` (handles, SIMD, falha, canal) | 4d |

## Medidas da fase

Regra do projeto: o mesmo comando antes e depois, nas duas ISAs ([SDD 07](07_desempenho_e_recursos.md)). Cada número entra no SDD 07 com a máquina e o acelerador.

| Medida | Como (proposta) |
| --- | --- |
| **Syscall nula** | `user/tests/null_syscall`: 1 M de `nop`, tempo por `rdtsc`/`cntvct`, mediana de 5, aquecimento descartado |
| **Entrada com KPTI** | O mesmo laço com `kpti=on` e `off` |
| **Criação de processo** | `process_create`, VMOs e mapeamentos de um programa mínimo, `thread_create`, `process_exit` e espera do `TERMINATED`, em laço |
| **Falta de página de usuário** | Três laços sobre 64 MiB: primeiro toque de escrita (zera um frame), leitura de página não tocada (página zero), escrita sobre a página zero (COW). Tempo por página |
| **Troca entre processos** | Dois processos passam a vez por canal com `object_wait_one` (o análogo do `lat_ctx`), com e sem PCID/ASID e com e sem KPTI |
| **Memória do menor processo** | Frames de um `hello`: tabelas de página, código, dados, pilha de usuário, pilha de kernel, tabela de handles. É a medida da meta de memória |
| **Mitigações** | Custo de cada uma da [seção](#mitigações-de-cpu), ligada e desligada |

### Números de referência (de outros sistemas)

Não se comparam com o RustOS: servem para dizer o que esperar.

| Número | Fonte | Grau |
| --- | --- | --- |
| Syscall nula: ferramentas `lat_syscall null` (mede `getppid()`) e `perf bench syscall basic`; **sem número de máquina nossa** | [lmbench](https://lmbench.sourceforge.net/man/lat_syscall.8.html), [LWN](https://lwn.net/Articles/674283/) | Ferramentas verificadas |
| 10 milhões de syscalls inválidas em 3,8 s no kernel 4.4 (cerca de 380 ns); hardware não informado | LWN | Secundária; **não usar como referência** |
| KPTI: cerca de 100 ciclos por troca de CR3, duas por syscall; perda de TLB "never exceed[s] 1%" | [Linux PTI](https://docs.kernel.org/arch/x86/pti.html) | Verificado (secundária): hardware não citado |
| `lat_ctx`: 1,2 a 1,5 µs pinado e 2,2 µs sem pino (Haswell, 2018) | blog | Secundária |
| `lat_ctx`: 1,4 µs e 1,8 µs (2000) | lkml | Primária |
| `INVLPG` cerca de 200 ciclos; `INVLPG` + `INVPCID` cerca de 550; uma página por `INVPCID`, 1 094; por CR3, 692 (Skylake desktop de 4 núcleos, sem HT) | [Hansen, lkml](https://lkml.iu.edu/1712.0/03977.html) | Primária, lida; uma CPU |
| Troca de PCID sem esvaziar: 200 a 230 ciclos (Sandy a Skylake) | PaX Team, via [openwall](https://lists.openwall.net/linux-kernel/2017/04/09/84) | **NÃO VERIFICADO** |
| Criação de processo: `lat_proc` (fork, exec) | lmbench | **NÃO VERIFICADO** (só a página de `lat_syscall` foi lida); os modelos diferem |
| Falta de página: nenhuma referência achada | — | — |

O zeramento do primeiro toque tem peso próprio: 160 ns por página só para zerar no aarch64
HVF ([DEBT-0020](../tech-debt/0020-zerar-pagina-pelo-memset-generico.md)).

**O que a QEMU não prova.** Sob TCG o TLB é de software e o custo de CR3, PCID e ASID não
representa o metal (**inferência**); KPTI e PCID só valem sob KVM, HVF (aarch64) ou metal
([DEBT-0014](../tech-debt/0014-x86-sem-salvaguardas-de-metal.md), [DEBT-0041](../tech-debt/0041-escalonador-do-x86-sem-medida-fora-do-tcg.md)). Os números do x86_64 sob TCG servem para
regressão entre PRs, não para comparar com o Linux.

## Dívidas que a fase paga, move ou abre

| Dívida | O que a fase faz |
| --- | --- |
| [DEBT-0012](../tech-debt/0012-excecao-de-usuario-so-para-a-cpu.md) Exceção de usuário só para a CPU | **Paga** na 4a (distinguir a origem, matar o processo) e fecha na 4d (canal de eventos) |
| [DEBT-0043](../tech-debt/0043-simd-sem-trap-de-uso-indevido.md) SIMD sem trap | **Paga** na 4d: `simd: true` em toda thread de usuário |
| [DEBT-0035](../tech-debt/0035-sve-sme-e-amx-sem-estado-por-tarefa.md) SVE, SME e AMX | O gatilho dispara, mas a fase **não liga** nada: o acesso trapeia e o processo morre. Reescrever o gatilho no PR da 4d |
| [DEBT-0020](../tech-debt/0020-zerar-pagina-pelo-memset-generico.md) Zerar página | O gatilho dispara (falta de página de usuário). Primeiro medir o caminho; pagar só se aparecer no número |
| [DEBT-0019](../tech-debt/0019-aleatorio-sem-fonte-de-hardware-fraco.md) Aleatório fraco | O ASLR é o primeiro consumidor de segurança: em CPU sem fonte de hardware o ASLR é fraco, e o boot o diz |
| [DEBT-0022](../tech-debt/0022-endereco-do-kernel-no-log-de-boot.md) Endereço do kernel no log | Nenhum endereço do kernel sai no `ProcessEvent` nem no `debug_write` |
| [DEBT-0023](../tech-debt/0023-aarch64-sem-wxn.md) aarch64 sem `WXN` | O gatilho dispara (primeiro mapeamento de usuário). Ligar na 4a, junto com `PageFlags.user`, se o Arm ARM confirmar o efeito sobre EL0 |
| [DEBT-0025](../tech-debt/0025-residuo-de-chave-na-pilha.md) Resíduo de chave na pilha | O gatilho dispara. Regra nova: estrutura que o kernel copia para o usuário é inicializada por inteiro, com o preenchimento zerado |
| [DEBT-0037](../tech-debt/0037-pilha-de-tarefa-sem-medida-de-pico.md) Pilha sem medida de pico | O gatilho dispara. A 4a mede o pico (syscall e falta de página) antes de fixar as 8 páginas |
| **A registrar no PR de cada fatia:** mitigações de CPU sem teste fora da QEMU (4e; gatilho: o primeiro boot em metal); ciclos entre filas de canais não coletados (4c; gatilho: o primeiro programa que os produza, ou os grupos de recursos da fase 9); imagem de usuário sem ASLR (4b; [decisão 5](18_fase_4_decisoes_pedidas.md#decisão-5--pie-de-usuário-agora-ou-depois)) | Cada uma com gatilho de evento, como manda o `AGENTS.md` |
| [DEBT-0011](../tech-debt/0011-gic-e-timer-so-na-cpu-0.md), [DEBT-0036](../tech-debt/0036-escalonador-com-estado-de-troca-global.md), [DEBT-0039](../tech-debt/0039-escalonador-de-uma-cpu.md) SMP | Não bloqueiam; o desenho põe a área por CPU de modo que o SMP só acrescente instâncias |

**O que fica de fora, e vira dívida com gatilho no PR em que o código nascer:** imagem
de usuário sem ASLR ([decisão 5](18_fase_4_decisoes_pedidas.md#decisão-5--pie-de-usuário-agora-ou-depois)), *shootdown* de TLB (segundo núcleo rodando thread do
mesmo processo), `vmar_protect`, sub-VMARs, TLS, `fork` e COW inteiro, multithread nativo
se a variante enxuta vencer, `rdtsc` por processo, AMX e VL do SVE por processo.

## Decisões e fontes

As decisões do dono, as do agente e as fontes lidas estão no
[SDD 18](18_fase_4_decisoes_pedidas.md). O diff dos guidelines que elas produzem já está em
[`guidelines/03`](../guidelines/03_camadas_e_dependencias.md).
