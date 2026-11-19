# SDD 17 — Processos, isolamento e syscalls (desenho da fase 4)

> **O que este capítulo responde:** como o kernel passa a rodar código em anel 3 e
> EL0 com espaço de endereçamento próprio, como o programa chama o kernel (entrada
> de syscall, handles, ABI nativa), como o primeiro programa chega à memória, e quais
> decisões do dono o desenho aplica.

**Estado: desenho aprovado no #9 (2026-11-18). As fatias 4a ([abaixo](#a-fatia-4a-como-entrou)) e 4b ([abaixo](#a-fatia-4b-como-entrou)) estão implementadas; da 4c, o crate puro `core/object` ([abaixo](#o-que-o-coreobject-torna-verdade-fatia-4c-parte-pura)) a parte 1, objetos, handles, canais, espera e canal de boot no kernel ([abaixo](#a-fatia-4c-parte-1-como-entrou)), a paginação sob demanda, a página zero e o *shootdown* de TLB ([abaixo](#a-fatia-4c-paginação-sob-demanda-e-shootdown-como-entrou)); e a parte 3, VMO, VMAR, processos, threads, o teto em bytes e o `init` com um filho ([abaixo](#a-fatia-4c-parte-3-como-entrou)); as demais, não.** A ABI de
syscall e o formato de executável são contrato que o mundo de fora enxerga (caso 6 do
[`AGENTS.md`](../../AGENTS.md)), e crate novo em `core/` e pasta nova de primeiro nível são
o caso 1; por isso o desenho nasceu num PR em Draft só com docs. O dono aprovou as nove
[decisões pedidas](#as-decisões-do-dono) sem mudança. O código vem em PRs seguintes, e
este capítulo passa a descritivo à medida que cada fatia entra: o que a 4a e a 4b tornaram verdade está
em [A fatia 4a, como entrou](#a-fatia-4a-como-entrou), [A fatia 4b, como entrou](#a-fatia-4b-como-entrou) [A fatia 4c, paginação sob demanda e *shootdown*, como entrou](#a-fatia-4c-paginação-sob-demanda-e-shootdown-como-entrou) e [A fatia 4c, parte 3, como entrou](#a-fatia-4c-parte-3-como-entrou), o **M1**, em que o shell lança os programas do initramfs ([abaixo](#o-m1-programas-pelo-shell-como-entrou)); e o resto do texto continua sendo desenho.

Fase 4 do [roadmap](08_roadmap.md). Pesquisa de origem, com as marcas de
verificação: `syscalls-processes.md` (2026-11-17). O modelo de controle de acesso
já está decidido: **modelo A** ([SDD 09](09_seguranca.md)), handles com direitos
que só diminuem, sem uid/gid no núcleo.

## Como ler as marcas

| Marca | Significa |
| --- | --- |
| **Verificado** | Lido numa fonte primária (spec, código do Linux, doc do projeto) nesta pesquisa; a fonte vem junto |
| **Verificado (secundária)** | Lido num espelho ou resumo de terceiro; confirmar antes de codar |
| **Verificado pelo Linux** | Posição de campo ou valor confirmado no código do Linux (`arch/arm64/tools/sysreg`, `pgtable-hwdef.h`, `arm_arch_timer.h`), que **não** é a especificação da Arm. Basta para escrever o desenho; a ARM ARM confirma antes do código |
| **NÃO VERIFICADO** | Não confirmado em fonte; **não** é premissa do desenho. Dito o que ler antes de codar |
| **Proposta** | Desenho do RustOS aprovado pelo dono no #9, sem fonte a citar. Vira fato quando o código da fatia entra |
| **Já é assim** | Fato do código de hoje (caminho dado) |

## Estado e escopo

### O que o código já dava à fase (antes da 4a)

| Fato | Onde | Consequência |
| --- | --- | --- |
| A GDT já tem dados de usuário (`0x18`) antes do código de usuário (`0x20`), "a ordem que o `sysret` espera" | `arch/x86_64/src/segments.rs` (**Já é assim**) | `IA32_STAR[63:48] = 0x10` serve; ver [x86_64](#x86_64-syscallsysret) |
| SMEP, SMAP e UMIP ligados; PAN no aarch64 | `platform::harden` ([SDD 13](13_memoria_virtual.md)) | O kernel já não executa nem toca página de usuário por engano |
| Metade baixa vazia: x86_64 com as 256 entradas de baixo sem uso; aarch64 com `TCR_EL1.EPD0` desligando o `TTBR0_EL1` | [SDD 13](13_memoria_virtual.md) | O espaço do usuário nasce nessa metade; no aarch64 o `EPD0` muda |
| UXN em toda folha do kernel no aarch64 | [SDD 13](13_memoria_virtual.md) | Página do kernel nunca executa em EL0 |
| `PageFlags` não tem bit de usuário; o mapeador recusa W+X | `core/hal/src/paging.rs` | `PageFlags` ganha `user`; o W^X de usuário vem de graça |
| `Fault` não diz de que anel veio | `core/hal/src/interrupt.rs` | Ganha `from_user` ([falhas de usuário](#falhas-de-usuário)) |
| `Context`/`switch` de thread do kernel, `simd` por tarefa, EEVDF | [SDD 15](15_tarefas_e_preempcao.md) | O processo de usuário é uma tarefa com mais estado |
| `MAX_TASKS = 32` (8 ociosas e 24 tarefas) e uma política `Composite<4, 32>` **por CPU**, no heap | `kernel/src/task.rs` ([SDD 15](15_tarefas_e_preempcao.md)) | Pouco para processos; a fase sobe a capacidade, e o custo agora **multiplica pelo número de CPUs** (a política de cada CPU cabe qualquer tarefa; 96 bytes por tarefa justa) |
| Pilha de falha do aarch64 em `SP_EL0` (o `vector_fault` faz `msr spsel, #0` e empilha lá; `install_secondary` grava a de cada CPU); EL1h com `SP_EL1` por tarefa | [SDD 15](15_tarefas_e_preempcao.md), [SDD 10](10_interrupcoes.md) | Em EL0, `SP_EL0` é a pilha do **usuário**: o `vector_fault` passa a pegar a pilha de falhas pelo bloco por CPU, que o `TPIDR_EL1` já aponta ([aarch64](#aarch64-svc)) |
| **SMP completo (#13):** um bloco `PerCpu` por núcleo (`IA32_GS_BASE` com `gs:[0]` = o endereço, e `IA32_KERNEL_GS_BASE` **zerado de propósito**, à espera do `swapgs`; `TPIDR_EL1` no aarch64); GDT, TSS e 3 pilhas IST por CPU, IDT compartilhada; IPI de reescalonar (`0xFD`, SGI 0) e de parar (`0xFE`, SGI 1); fila `Composite` por CPU, migração, `on_cpu` e `percpu::pinned` | [SDD 15](15_tarefas_e_preempcao.md), [SDD 10](10_interrupcoes.md) | A fase 4 **nasce em N núcleos**: tudo o que ela guarda por CPU vive no `PerCpu`; uma tarefa de usuário migra como as outras. O *shootdown* de TLB (dívida 0055) foi o gatilho que a fase disparou e a 4c pagou |
| `#DF`, NMI e `#MC` têm IST (IST1 a IST3); `#DB` não | `arch/x86_64/src/traps.rs` | `#DB` ganha IST própria e entrada paranoica ([exceções do anel 3](#exceções-vindas-do-anel-3)) |
| Janela de pilhas só de avanço | [SDD 13](13_memoria_virtual.md) | Criar e matar processos em laço esgota endereço; precisa reciclar |

### O que a fase entrega, por fatia

A fase é grande demais para um PR. Cada fatia é um PR com `make pr` verde e um
número novo no [SDD 07](07_desempenho_e_recursos.md).

| Fatia | Entrega | Pronto quando |
| --- | --- | --- |
| 4a (**feita**) | Entrada de syscall e saída para anel 3/EL0 nas duas ISAs; `PageFlags.user`; `Fault.from_user`; `#DB` com IST; cópia usuário↔kernel; espaço de **um** processo, sem objetos; **limpeza da pilha do kernel na volta ao usuário** (pagou a dívida do resíduo do ChaCha20 na pilha); a troca para o par `swapgs` e os MSRs de syscall em cada CPU; `nop`, `clock_get_monotonic`, `random_get`, `process_exit` e **`debug_write(ptr, len)` provisória, sem handle**; um programa embutido no kernel só para esta fatia | `hello` imprime de anel 3 e de EL0; falha de usuário mata o processo e o kernel segue (pagou a dívida da exceção de usuário só para a CPU) |
| 4b (**feita**) | `core/cpio`, `core/elf`; a árvore de regiões em `core/mm/src/vmar.rs`, **interna ao kernel** (o carregador mapeia frames direto, sem objeto VMO nem VMAR e sem handle); módulo do Limine; `user/` (runtime e `init`); o xtask monta o initramfs | O `init` do initramfs roda, imprime pelo `debug_write` provisório e sai; o smoke exige a linha dele (**no modo de teste**: [DEBT-0087](../tech-debt/0087-init-so-no-boot-de-teste.md)) |
| M1 (**feito**) | O shell (`ls`, `run`, `ps` com a coluna `pid`) lista os programas do initramfs e os roda como processos isolados **no boot normal**, na imagem `release-small`; quatro programas de demonstração em `user/` (`hello`, `crash`, `sysinfo`, `count`); o smoke digita a sessão no shell | O dono sobe a imagem, digita `ls`, `run hello`, `run crash` e `ps`, e vê o sistema sobreviver à morte contida ([abaixo](#o-m1-programas-pelo-shell-como-entrou)). Paga a metade do [DEBT-0087](../tech-debt/0087-init-so-no-boot-de-teste.md) que dizia respeito ao carregador |
| 4c (**feita**: a parte 1, handles, canais, espera e canal de boot; a paginação sob demanda e o *shootdown*; e a parte 3, VMO, VMAR, `process_create`, `thread_create`, `thread_exit` e `process_kill`, o teto em bytes e o `init` com um filho, [abaixo](#a-fatia-4c-parte-3-como-entrou)) | `core/object`; handles e direitos; **VMO e VMAR como objetos**; `debug_write` ganha o argumento `console` (**feito**); **página zero** e **falta de página de usuário resolvida** (**feitas**, [abaixo](#a-fatia-4c-paginação-sob-demanda-e-shootdown-como-entrou)); ASLR; `process_create`, `thread_create`, `process_kill`, `object_wait_one`, canais; ***shootdown* de TLB** e espera da CPU que ainda roda o espaço na morte do processo (**feitos e pagos**: dívidas 0055 e 0085); o `init` recebe os handles pelo canal de boot (**feitos**) | Um processo cria outro, que escreve no console por um handle que recebeu (**atingido**: o `init` cria o `child`) |
| 4d | SIMD de usuário, canal de eventos do processo, teste de `xmm0`/`v0` entre processos | Dois processos preservam `xmm0`/`v0`; o culpado de uma falha chega ao pai (**paga** [DEBT-0043](../tech-debt/0043-simd-sem-trap-de-uso-indevido.md)) |
| 4e | KPTI, PCID/ASID com a regra de geração entre CPUs, mitigações de CPU ([seção](#mitigações-de-cpu)), relatório no boot. **Parte 1 (feita):** detecção de vulnerabilidade e de recurso nas duas ISAs, a linha `mitigations:`, a chave `kpti=` (só no relatório) e o clamp do Spectre v1. **Parte 2:** o KPTI, o PCID/ASID na troca e o IBPB na troca | Linha `mitigations:` no boot; custo de cada uma medida |
| 4f | As medidas da fase (inclui a falta de página que zera, [DEBT-0058](../tech-debt/0058-zerar-pagina-no-x86-sem-medida-em-hardware.md)) ([abaixo](#medidas-da-fase)) e este capítulo virando descritivo | Fecha a fase: "syscall nula medida; processo que toca o kernel morre sem derrubar nada" |

**A ABI da 4a e da 4b era provisória.** `debug_write` ganhou o argumento `console` na parte 1 da 4c e todas as chamadas já implementadas têm a forma final. A ABI nativa é experimental
até o fim da fase 5, então a mudança não quebra contrato.

## A fatia 4a, como entrou

**Descritivo.** O que a 4a (PR `feat/phase4a-user-entry`) tornou verdade, nas duas ISAs. O resto do
capítulo segue como desenho. Os números estão em
[SDD 07](07_desempenho_e_recursos.md#syscall-nula-e-limpeza-da-pilha-fatia-4a).

| Peça | Onde mora |
| --- | --- |
| Contrato numérico (`Syscall`, `Status`, `Rights`, `Handle` de 64 bits, `ProcessEvent`, mensagem de boot, `USER_MIN`, `USER_TOP`, filtro do `debug_write`) | `core/hal/src/abi.rs`, com teste de layout e de cada número |
| `PageFlags.user`, `Fault.from_user`, `Handlers.syscall` e `Handlers.leave_user` | `core/hal/src/{paging,interrupt}.rs`; o mapeador recusa folha de usuário fora de `[USER_MIN, USER_TOP)` e folha de kernel na metade baixa |
| `UserRange` e a metade de usuário das tabelas (`adopt_top_entries`, `release_user_half`) | `core/mm/src/{user,tables}.rs` |
| x86_64: `SYSCALL` e `SYSRET`, `swapgs`, entrada paranoica de NMI, `#DF`, `#MC` e `#DB` (IST4), `usercopy`, `CpuPrefix`, `sysret` puro | `arch/x86_64/src/{traps,user,usercopy,prefix,sysret}.rs` |
| aarch64: vetores de EL0, `SVC`, `eret`, `usercopy` com `ldtr`/`sttr`, `CNTKCTL_EL1`, `TTBR0_EL1`, `WXN` | `arch/aarch64/src/{exception,user,usercopy,mmu,prefix}.rs` |
| Programa de usuário de teste (`global_asm!` em `arch/<isa>`, atrás da feature `selftest`) | `arch/<isa>/src/user_blob.rs` |
| Processo, espaço de usuário, syscalls, limpeza da pilha | `kernel/src/{process,uspace,syscall,scrub}.rs`; o autoteste em `kernel/src/process/selftest.rs` |
| `check-layers` com `user/` e `arch/<isa>-user` | `xtask/src/checks/layers.rs` (o teste de que o `kernel/` não liga `arch/x86_64-user` nasceu vermelho) |

**O que a 4a faz, por ISA.**

| | x86_64 | aarch64 |
| --- | --- | --- |
| Entrada | `SYSCALL` no `LSTAR`; `EFER.SCE`, `STAR`, `LSTAR` e `FMASK` gravados em cada CPU em `load_cpu`; o quadro é o mesmo `TrapFrame` de 22 palavras das exceções | `svc #0` no vetor 8; o quadro de 288 bytes ganhou `sp_el0` e `tpidr_el0` |
| Pilha de entrada | `gs:[kernel_rsp]` e o `RSP0` da TSS desta CPU, escritos na troca para a tarefa de usuário | `SP_EL1`, que a primeira volta põe no topo da pilha da tarefa |
| Saída | `SYSRET` se a conferência passa (`RCX == RIP`, `R11 == RFLAGS`, seletores, `RIP < USER_TOP`, sem `RF` nem `TF`), senão `IRETQ` | `eret` |
| `swapgs` | Um por transição de anel, pelo `CS` salvo; NMI, `#DF`, `#MC` e `#DB` decidem pelo sinal de `IA32_GS_BASE` | não há |
| Cópia | `rep movsb` entre `stac` e `clac` (só com SMAP), com *fixup* no `rip` da instrução | `ldtrb`/`sttrb` com o PAN ligado, com *fixup* no `elr` |
| Espaço | `CR3` por processo, com as 256 entradas do kernel copiadas por ponteiro; sem PCID, só se escreve o `CR3` quando a raiz muda | `TTBR0_EL1` por processo, uma raiz vazia para tarefa de kernel, `EPD0` limpo, ASID 0 e `tlbi vmalle1` a cada troca de raiz |
| Contador em usuário | `rdtsc` (`CR4.TSD = 0`) | `CNTKCTL_EL1.EL0VCTEN` |

**Divergências do desenho, todas sem mudar contrato** (decisões do agente):

| Decisão | O que ficou | Por quê |
| --- | --- | --- |
| Syscall roda mascarada | O despacho roda com a IRQ mascarada, como uma exceção | Nenhuma chamada da 4a bloqueia. A parte 1 da 4c **não ligou** a IRQ: `object_wait_one` cede a CPU como `sleep_until` ([DEBT-0081](../tech-debt/0081-debug-write-de-512-bytes-com-a-irq-mascarada.md)) |
| `IRETQ` que falha | A saída para o usuário confere `RIP` e `RSP` canônicos no quadro e mata o processo se algum não for, em vez do *fixup* nos vetores 11, 12 e 13 | `RSP` não canônico é a única causa que o usuário controla; o *fixup* é a [DEBT-0077](../tech-debt/0077-iretq-sem-fixup-de-falha-fora-do-rip-e-do-rsp.md) |
| `#DB` do usuário | O stub da IST4 faz `swapgs`, copia o quadro para a pilha da tarefa e segue como entrada normal; o processo morre na pilha da tarefa | Como o SDD manda; exercido pelo caso `fault_trap` (`TF` ligado) |
| Pilha de falhas do aarch64 | O `vector_fault` lê o topo pelo `TPIDR_EL1` e guarda o `x0` no `TPIDRRO_EL0`; exceção síncrona de EL1h segue na pilha de falhas, sem o teste de estouro da pilha da tarefa | O `SP_EL0` agora é a pilha do usuário; a ordem do Linux fica na [DEBT-0078](../tech-debt/0078-aarch64-excecao-do-el1h-na-pilha-de-falhas.md) |
| `T0SZ` do aarch64 | Fica em 16 (48 bits), o que o Limine deixa; o mapeador só mapeia abaixo de `USER_TOP` | Mudar para 17 não ganha nada, e o código de serial precoce já confere o 16 |
| Limpeza da pilha | A volta ao usuário repinta a pilha do kernel abaixo do quadro com `STACK_PAINT`, varrendo até 128 bytes limpos; o `arch/` repinta os 256 bytes logo abaixo sem usar pilha | Custo medido; a heurística é a [DEBT-0073](../tech-debt/0073-limpeza-da-pilha-so-ate-128-bytes-de-limpo.md) |
| Onde mora o programa de teste | Blob de `global_asm!` em `arch/<isa>/src/user_blob.rs`, na seção `.text` do kernel, copiado para uma página de usuário em `0x40_0000` | O `asm!` só mora em `arch/` (regra 4 do guidelines 04) e o `kernel/` nunca liga `arch/<isa>-user` (regra 12); código de teste fica atrás da feature `selftest` |
| `simd` | A tarefa de usuário nasce com `simd: true` | Sem área de estado, o `xmm0` e o `v0` vazariam entre tarefas; a 4d fecha o resto |
| `kernel_half_in_sync` | `Process::load` confere que as 256 entradas altas da raiz nova são as da raiz do kernel e recusa o processo se não forem | Entrada nova do kernel depois da cópia dessincronizaria as raízes |
| `AC` (x86_64) | **Toda** entrada por vetor (a comum, a paranoica e a do `#DB`) faz `clac` **antes** de olhar a origem, se a CPU tem SMAP (sem SMAP, `clac` é `#UD`), como o `ASM_CLAC` do Linux; o `iretq` restaura o `AC` do quadro, então limpar é seguro. Vale para o usuário que liga `AC` por `popf` e para a exceção ou NMI que chega dentro da janela `stac`…`clac` da cópia (um `#PF` recuperável, por exemplo). A entrada por `SYSCALL` já tem o `AC` no `FMASK`. A QEMU limpa o `AC` na IRQ mas não na exceção, e o autoteste pega a exceção: de usuário (`privileged`) e de kernel (o `#PF` do `hello` num ponteiro não mapeado, contado em `usercopy::fixup_stats`) | Sem o `clac`, o tratador e a tarefa de kernel retomada rodariam com o SMAP desarmado. **Sem SMAP na CPU** (`RUSTOS_CPU=qemu64`: `protections: +wp +nx -smep -smap -umip`) a entrada não faz `clac` e o `AC` ligado no anel 0 não tem efeito: o autoteste só conta `AC` ligado no kernel (`AC_LEAKS`, `check_ac`) quando o SMAP está ligado, e o caso `privileged` passa |
| Falta na cópia | O *fixup* só vale se o endereço da falta está abaixo de `USER_TOP`; falta em endereço do kernel durante a cópia é fatal | Um erro do kernel não vira `OutOfRange` |
| Código de usuário no aarch64 | Escrever página executável pelo mapa direto faz `dc cvau`, `ic ivau`, `dsb ish`, `isb` (`arch/aarch64/src/cache.rs`) | O `CTR_EL0` do HVF é `IDC=1`, `DIC=0`: sem isso o usuário podia executar bytes velhos |
| Filtro do `debug_write` | Só ASCII imprimível, `\n` e `\t` passam; todo outro byte (inclusive os de UTF-8, que escondem o CSI de 8 bits, U+2028 e o bidi) vira `?`; cada linha sai com `[user] ` na frente e a saída sempre termina em `\n`. O xtask só aceita marcador do kernel no **começo** da linha (`[rustos] …`, sem `\r`) | O usuário não imita uma linha `[rustos]` do smoke nem encerra o boot cedo com um `[rustos] boot ok` no meio da linha dele |

**Medido na QEMU 11.1.3** (o que o desenho tinha como NÃO VERIFICADO e a pesquisa só tinha de fonte
secundária):

| Item | Resultado |
| --- | --- |
| PAN e `LDTR`/`STTR` (aarch64) | Com `PAN` ligado, a cópia por `ldtrb`/`sttrb` funciona (o `debug_write` do `hello` lê o buffer do usuário) e um `ldr` comum do kernel numa página de usuário dá falta de permissão em EL1. A rotina **não** abre a janela do PAN |
| SMAP (x86_64) | Um `mov` comum do kernel numa página de usuário dá `#PF` de proteção; a cópia entre `stac` e `clac` funciona |
| `WXN` sobre EL0 (aarch64) | Uma página de código de usuário mapeada de propósito como RWX (o mapeador foi desligado só nesse teste, que não está no código) executa sem o `WXN` e dá falta de instrução em EL0 com ele. O `WXN` vale para EL0. Ligado depois de `Vm::activate` em cada CPU |
| `SVC` vindo de EL0 | `EC = 0x15`; o vetor 8 o despacha pelo `x8` |
| Privilegiada em EL0 | `msr ttbr0_el1` dá `EC 0x00` (instrução desconhecida), e `mrs` de contador ou timer que o `CNTKCTL_EL1` nega dá `EC 0x18`; o decodificador trata os dois como instrução ilegal. `brk` de EL0 dá `EC 0x3C` e mata o processo |
| `hlt` e `TF` em anel 3 | `hlt` dá `#GP`; `popf` com `TF` dá `#DB` na IST4, com o processo morto na pilha da tarefa |
| `SYSRET` | Passa o autoteste inteiro; o `IRETQ` fica de reserva e se pode ligar e desligar pelo autoteste (número no SDD 07) |

**O que o autoteste confere** (`make smoke`, modo de teste, nas duas ISAs): o `hello` em cada CPU
ativa (`debug_write`, `clock_get_monotonic`, `random_get`, `process_exit`; o número desconhecido e
`thread_exit` devolvem `NotSupported` e o processo segue); leitura de
página do kernel; instrução privilegiada com `AC` ligado (o `#GP` chega ao kernel com `AC` limpo); `TF` ou `brk`; `sp` inválido (o kernel não cai ao voltar); `NT` e `AC` ligados por `popf` com syscalls e IRQ durante 8 ms (o processo sobrevive e o kernel nunca roda com `AC`); ponteiro de usuário não mapeado em `debug_write` (`OutOfRange`, o processo segue);
a limpeza da pilha (2 KiB plantados na pilha do kernel somem com a limpeza e aparecem sem ela).
Depois de cada falha o kernel segue e o caso seguinte roda.

### Divergências em relação ao roadmap

O [SDD 08](08_roadmap.md) descreve a fase 4 antes deste desenho. Três pontos mudam. O
roadmap muda no PR de cada fatia que torna o ponto verdade, junto com o código.

| O roadmap diz | O desenho da fase 4 diz | Por quê |
| --- | --- | --- |
| "VFS mínimo, initramfs e tmpfs" | Sem VFS na fase 4: o initramfs é um arquivo lido por quem o recebe como VMO; VFS e tmpfs entram na fase 5, com a tabela de descritores | O modelo nativo não tem caminho: tem handle. VFS existe para a ABI Linux |
| "sinais mínimos" | Sem sinais POSIX na ABI nativa; falha vira mensagem num canal ([decisão 6](#decisão-6--falha-de-usuário-canal-de-eventos-ou-sinal-síncrono)) | Sinal exige o sigframe, e o sigframe exige decidir antes o pedido de AMX e o VL do SVE ([SDD 08](08_roadmap.md#decisões-futuras-casos-do-agentsmd)); sem sinal, as duas decisões esperam a fase 5 |
| "COW no fork" | COW só a partir da página zero compartilhada; `fork` e o COW inteiro vêm com a ABI Linux (fase 5) | A ABI nativa não tem `fork`: cria processo vazio e o preenche |

## O que o `core/object` torna verdade (fatia 4c, parte pura)

**Descritivo.** O crate `rustos-object` (decisão 7) implementa a parte de [Modelo de objetos e
handles](#modelo-de-objetos-e-handles) que não precisa de kernel: roda e é testado no host, sem
`unsafe`, só com `core` e o módulo `abi` do `hal`. A ligação com os objetos reais, os locks, a cópia
de usuário e as syscalls é da 4c (`kernel/`). Os números estão em
[SDD 07](07_desempenho_e_recursos.md#tabela-de-handles-e-canal-fatia-4c-medido-no-host).

| Peça | O que é | Arquivo |
| --- | --- | --- |
| `HandleTable<T, S>` | Valor `geração << 16 \| (índice + 1)`; geração de 48 bits que sobe a cada reuso; o slot que chegaria a `2^48 - 1` é aposentado (2^48 - 1 usos por slot); lista livre FIFO; teto de 4 096 handles (usados mais reservados) e de 65 534 slots; `insert`, `get`, `get_of`, `remove`, `duplicate`, `replace`, `reserve`/`commit`/`cancel`, `check_array`/`take_array`, `drain` | `table.rs` |
| Slot de 32 bytes | `meta: u64` (geração nos 48 bits baixos, estado nos altos), `rights: u32`, elo da lista livre `u32` e `Option<T>`; com o `T` do kernel (um enum de seis `Arc`s, 16 bytes e nicho na tag) o slot mede 32, conferido por `size_of` no teste. Um `T` sem nicho daria 40 | `table.rs` |
| Direitos | `parse` (bit reservado é `InvalidArgs`), `require` (`AccessDenied`), `reduce` (subconjunto, senão `InvalidArgs`) e o módulo `required` com o que cada chamada exige, conferido contra as tabelas deste capítulo | `rights.rs` |
| Contagem de handles | `HandleCount` atômico no objeto: sobe em `commit`/`duplicate`, desce em `close`. O handle em trânsito (`Transit`) segue contado; só `close` o desconta. `Object::on_handle_closed` roda **depois** de soltar o lock da tabela; o padrão chama `on_zero_handles` (Process, Thread, VMO) ou, numa ponta de canal, a checagem de alcançabilidade | `object.rs` |
| `close` e `Worklist` | Fechamento em laço com lista de trabalho, sem recursão: a morte de uma ponta empurra os handles da fila dela, e o laço os fecha | `object.rs` |
| `Channel<T>` | Um par de pontas com a fila de cada uma (16 mensagens, 1 024 bytes, 8 handles), `own_in_queue` por ponta, `reserve`/`commit`/`cancel` da escrita, `peek`/`pop_if` da leitura, `on_handles_changed` (a regra do ciclo) e `signals` (`READABLE`, `WRITABLE`, `PEER_CLOSED`, por nível) | `channel.rs` |
| `channel_write`, `channel_read` | As ordens do SDD, em funções sobre `Lock<HandleTable>` e `Object::with_channel`; a falha nunca perde mensagem nem handle | `transfer.rs` |

### Ordem de lock (lacuna L3)

Duas ordens valem no kernel; o `core/object` as obedece e o teste de threads as exercita.

| Regra | Texto |
| --- | --- |
| Ordem global | **Tabela do processo, depois o lock do par de pontas.** Nunca o contrário, e nunca dois pares ao mesmo tempo. O alocador entra na ordem: **tabela, depois o heap, depois os frames** (o `try_reserve` que cresce os slots roda com a tabela presa e pode pedir um frame ao crescer o heap); o lock do par nunca aloca |
| Um lock por par | O `Channel` guarda as duas pontas e as duas filas: A→B e B→A pegam o mesmo lock |
| `channel_write` | Segura a tabela do escritor do começo ao fim e toma o lock do par duas vezes, só por dentro dela: reservar (passo 2) e *commit* (passo 5). Entre as duas, nenhum outro lock é tomado |
| `channel_read` | Passo 1: valida `ch` sob a tabela e, dentro dela, espia sob o par. Passo 2: reserva slots sob a tabela. Passo 3: **sem lock**. Passo 5: sob a tabela, retira sob o par e instala os handles |
| Fechar | `handle_close`, o descarte do array e `drain` removem **sob** a tabela e fecham **depois de soltá-la**, sem lock nenhum. `on_handle_closed` toma só o lock do par da ponta; a morte **só marca** a ponta (`on_handles_changed`, sob o lock do par). Solto o lock, `drain_dead` retira as mensagens da ponta morta uma a uma (`pop_dead`, cada retirada sob o lock do par) e empurra os handles para a lista de trabalho, **sem lock preso**; fechá-los (que pode tomar outro par) acontece depois |
| Ganchos | `on_zero_handles` roda sem lock. `on_end_died` roda sem o lock do par; na escrita (morte da ponta de destino pelo ciclo) roda ainda com a tabela do escritor, então o código dele só pode tomar locks que ficam abaixo da tabela, como o do escalonador. A fila da ponta que morreu na escrita se esvazia depois de soltar a tabela (`channel_write` leva a ponta no resultado) |
| Contagem | `HandleCount` é atômico e fica fora do lock do par. A checagem de morte **relê** `handle_count` sob o lock do par, no momento da checagem; o `remaining` que `release` devolveu, sem lock, pode estar velho (outra thread duplica ou escreve um handle da ponta entre o desconto e o lock) e **não** serve para decidir. Quando `handle_count == own_in_queue` sob o par, ninguém mais alcança a ponta e a contagem não sobe. Teste: `a_stale_remaining_count_does_not_kill_an_end_that_is_still_reachable` |

### O que fechou das lacunas

| # | Parte pura, feita e testada em `core/object` | O que fica para a 4c |
| --- | --- | --- |
| L1 | `Channel::commit` reconfere a ponta oposta sob o lock do par e devolve a mensagem se ela morreu; `channel_write` fecha os handles dela e responde `PeerClosed` (teste `the_peer_dying_between_reserve_and_commit_…`; e threads que fecham o par durante a escrita) | **Feito** (a ligação com o `IrqLock`; o teste de SMP é o smoke com `handles` e `init` nos núcleos) |
| L2 | `channel_read` repete desde o passo 1 se a frente mudou e **cancela** as reservas, e `cancel` sobe a geração: o valor reservado e solto não volta (testes do `a_competing_read_…` e do modelo, que confere que nenhum valor reaparece) | **Feito** (`copy_to_user` real no passo 3; o programa `handles` o exerce com ponteiro ruim) |
| L3 | A tabela acima | **Feito na parte 1 da 4c** (`IrqLock` por processo e por par, na ordem) |
| L4 | `reserve` → `commit` ou `cancel` na tabela; a reserva desfeita não deixa handle, não conta em `len` e consome a geração (o modelo cria canais com 20% de desistência) | **`channel_create` feito** (reserva, cópia, confirmação); `process_create` vem na parte 3 |

### Decisões do agente

Nenhuma é dos seis casos do [`AGENTS.md`](../../AGENTS.md); cada uma diz o que a faria subir.

| Decisão | O que ficou | Contingência |
| --- | --- | --- |
| Memória dos slots | A tabela é genérica sobre `SlotStorage<T>`, que o kernel implementa com um `Vec` e `try_reserve` (falta de memória é `NoMemory`, não aborta). O crate traz `FixedSlots<T, N>`, sem alocação, que serve ao teste e a tabelas pequenas. `core/` não chama o alocador (guidelines 05) | Se o kernel preferir `Vec` dentro do crate, é exceção ao guidelines 05: caso 3 |
| Custo do `Vec` no kernel | A tabela cresce de um em um slot; o `Vec` dobra a capacidade, então a memória passa até do dobro do necessário (pior caso 256 KiB com 4 096 handles) | Medir na 4c; um `Vec` com crescimento fixo se doer |
| Quem conta | O objeto traz o `HandleCount`; a tabela só o sobe em `commit` e `duplicate` e desce em `close`. `commit_moved` e `remove` não mexem, porque o handle só muda de lugar | — |
| `replace` | Segue o SDD e o Zircon lido (com a inferência marcada): o handle velho **sempre** morre se existe, inclusive no erro (`InvalidArgs` por direito maior; `NoMemory` se faltar slot). O fechamento volta em `ReplaceFailed::closed`. Não é "tudo ou nada" | — |
| Limites do canal | Mais de 1 024 bytes ou 8 handles é `InvalidArgs` e **consome** o array (a regra geral só poupa `ShouldWait` e a falha ao ler o array). O Zircon devolve `OUT_OF_RANGE` (lido em `channel_write`); a tabela de erros do RustOS reserva `OutOfRange` a ponteiro e intervalo de usuário e define `InvalidArgs` como "argumento fora do domínio", então o código segue a tabela, sem status novo. Mensagem vazia e sem handle é válida. O kernel limita a cópia do array a 64 entradas (o teto do Zircon); acima disso não sabe quais handles são e responde `InvalidArgs` sem consumir | Se o dono quiser `OutOfRange`, muda a tabela de erros e aqui |
| Bytes da mensagem | O kernel os copia para um `Object::Bytes` (um `Arc<[u8]>`) **antes** de chamar `channel_write`; a espiada da leitura clona esse `Arc`, e por isso o passo 3 copia sem lock | — |
| Fila inline | Cada ponta guarda 16 mensagens inline; o par mede 7 256 bytes ([DEBT-0091](../tech-debt/0091-fila-do-canal-com-armazenamento-inline.md)) | — |
| Lista de trabalho | `Worklist` é trait, e `push` não falha. A morte da ponta **só marca** sob o lock do par; os handles da fila dela vão para a lista **depois** de soltá-lo (`drain_dead`), então o `push` nunca roda com lock preso e a lista do kernel é um `Vec` que cresce. Era requisito da 4c que o `push` não alocasse com o lock do par preso; a mudança de lugar da retirada o cumpre sem lista intrusiva nem reserva por canal (7 KiB por par a mais). O teste `Unlocked` confere que nenhum `push` roda com o lock da tabela ou do par preso | — |
| Número de sequência | `seq` é `u32` por ponta, com `wrapping_add`. Só separa a mensagem da frente da que a substituiu durante uma cópia: repetir um valor pede 2^32 mensagens escritas e lidas naquela fila entre o passo 1 e o passo 5 de **uma** leitura, com a fila em 16, o que é impraticável | — |
| Identidade da ponta | `Object::channel_end` devolve `EndId { channel, side }`, sem lock; a regra "a própria ponta não viaja" compara essa identidade, não o valor do handle | — |
| Ordem dos erros do array | Para cada handle, na ordem: existe, não repete, tem o direito, não é a própria ponta; o primeiro erro vence | — |
| `ShouldWait` e o array | Fila cheia devolve `ShouldWait` antes de olhar o array, como o SDD manda: um erro no array só aparece na repetição | — |
| `locate` e o valor zero | `Handle::INVALID` e todo valor sem índice dão `BadHandle`, sem depender de o chamador ter usado `handle_from_raw` | — |
| Teste de entrelaçamento | O SDD pedia embaralhar passos de threads do mesmo processo e falhar **sem o lock**. Sem `unsafe`, a tabela só se alcança por `&mut` dentro de `Lock::with`, então "sem lock" não compila. Em troca: a repetição da leitura é testada com uma leitura concorrente injetada entre a cópia e o *commit* (a janela real, em que a tabela está solta), e há threads de verdade (escritor, leitor e *churn* de handles na mesma tabela; fechar o par no meio da escrita) | Se a 4c quiser o embaralhador, ele cabe sobre `Lock` |

### Limite que o modelo de teste mostra

O teste de modelo (milhares de operações aleatórias contra um mapa e um ponto fixo da regra de
morte da ponta) confere, a cada 25 operações, o conteúdo das tabelas, o `handle_count` de todo
objeto, a morte das pontas, as filas e `own_in_queue`. Ao fim, fechar todas as tabelas **não**
zera as contagens que estão em ciclos entre filas: é o limite documentado e a
[DEBT-0090](../tech-debt/0090-ciclos-entre-filas-de-canais-nao-coletados.md).

## A fatia 4b, como entrou

**Descritivo.** O que a 4b (PR `feat/phase4b-init`) tornou verdade, nas duas ISAs. Os números estão em
[SDD 07](07_desempenho_e_recursos.md#processo-mínimo-e-carregador-do-init-fatia-4b).

| Peça | Onde mora |
| --- | --- |
| Auxv (`AT_*`), `StartInfo`, `InitialStack` (monta `argc`, `argv`, `envp`, auxv e `AT_RANDOM` no topo da pilha, `sp` alinhado a 16) e o teto de `p_memsz` de um executável (`EXEC_MAX_LOADS_BYTES`, 64 MiB) | `core/hal/src/abi.rs`, com teste de layout e de cada tag |
| Carregador: acha o arquivo (`core/cpio`), valida (`core/elf`, com os `Limits` de `hal::abi`), copia e mapeia cada `PT_LOAD`, registra as regiões no `Vmar` (`core/mm`) e monta a pilha com guarda e topo sorteado | `kernel/src/loader.rs` |
| `Source::{Elf, Flat}` (o `Flat` é o blob da 4a, atrás de `selftest`), `Spawned`, a árvore de regiões em `Process` | `kernel/src/process.rs` |
| `UserSpace::map_at` (o `p_vaddr` do segmento não é de página: o começo da primeira página e a cauda ficam zero) e `SpaceStats` | `kernel/src/uspace.rs` |
| O lançamento do `init`, a espera e a conferência do código de saída | `kernel/src/init.rs` (só com `selftest`) |
| `_start` (alinha o `sp`, zera o frame pointer, chama `rustos_user_main(pilha, handle)`) e o stub de `syscall`/`svc` | `arch/{x86_64,aarch64}-user` |
| Ponto de entrada em Rust, leitor do auxv (`Env`), `println!`, wrappers seguros das syscalls da 4a, pânico, alocador de avanço | `user/rt` (o único `unsafe` de usuário) |
| `init` (`forbid(unsafe_code)`) e `big` (programa de teste de 1,3 MiB, `deny(unsafe_code)` como todo `user/tests/*`) | `user/init`, `user/tests/big` |
| Linker scripts | `user/link/{x86_64,aarch64}.ld` |
| `cargo xtask user`, o initramfs da imagem (`init`, mais `big` com `selftest`), as linhas do smoke | `xtask/src/{user,image,initramfs,qemu}.rs` |

**O que o `init` confere** (e sai com 0 se tudo bate): `argc` é 0; `sp` alinhado a 16; handle de boot `0` (na 4b; passou a `1` na parte 1 da 4c);
`AT_PAGESZ` 4096; `AT_ENTRY` igual ao endereço do `_start`; `AT_PHDR` aponta para uma tabela de `AT_PHENT = 56` com ao menos
dois `PT_LOAD`; `AT_HWCAP` e `AT_HWCAP2` zero; `AT_RANDOM` presente e não zerado; `clock_get_monotonic` positivo e não
decrescente; `random_get` de 16 bytes diferentes dos de `AT_RANDOM`. Imprime uma linha, `init: ok (...)`, que o smoke exige com o
prefixo `[user] `.

**Divergências do desenho** (decisões do agente, nenhuma muda contrato):

| Decisão | O que ficou | Por quê |
| --- | --- | --- |
| O `init` só roda no modo de teste | O boot normal não o lança; **desde o M1 o carregador está na `release-small`** e o shell lança qualquer programa do initramfs, o `init` inclusive ([DEBT-0087](../tech-debt/0087-init-so-no-boot-de-teste.md), reescrita) | Na 4b o `init` é um autoteste, e o guidelines 05 manda o que roda tarefa para o modo de teste. Experimento com ele no boot normal: +1 a 3 ms no aarch64 em HVF (2 ms para 4–7) e cerca de +10 ms no x86_64 em TCG, sem ganho para o usuário. O boot normal sem initramfs ou sem `init` segue como antes |
| `-C code-model=small` com `-C lto=off` no x86_64 | O `core` pré-compilado do alvo vem em `code-model=kernel`; com o LTO fat dos perfis `release`/`release-small` o LLVM recusa o módulo (`linking module flags 'Code Model': IDs have conflicting values`, `failed to load bitcode`). O `xtask user` liga `-C lto=off` no x86_64 ([DEBT-0084](../tech-debt/0084-usuario-do-x86-sem-lto-por-causa-do-code-model.md)) | O `small` é a decisão aprovada; o `init` custa 17 600 bytes sem LTO contra 15 176 com o modelo `kernel` e LTO. **O item "Por ler" do `small` fecha**: o `big` (1,3 MiB de arquivo, 2,3 MiB mapeados, `.rodata`, `.data` com cauda de 400 bytes e `.bss` de 1 MiB) liga em `0x40_0000` e roda nas duas ISAs. O `small` e o `kernel` geram o mesmo código a 4 MiB (17 920 bytes nos dois, sem LTO), então o teste continua sem distingui-los |
| Layout do linker | Cabeçalhos e `.rodata` no primeiro `PT_LOAD` (R), `.text` (RX) numa página a partir de `0x40_1000`, `.data` e `.bss` (RW) juntos num `PT_LOAD` com `-z norelro`; `AT_PHDR` aponta `0x40_0040` | O primeiro `PT_LOAD` com os cabeçalhos faz o `AT_PHDR` existir; sem `norelro` o `.bss` virava um `PT_LOAD` de `p_filesz` 0 |
| Pilha | 16 páginas mapeadas na criação, com uma página de guarda abaixo e outra acima e o início sorteado com `aslr_base` na janela `[0x6000_0000_0000, 0x6100_0000_0000)` (28 bits de página, o intervalo cheio); a imagem fica em endereço fixo ([DEBT-0083](../tech-debt/0083-imagem-de-usuario-sem-aslr.md)). Mapeada inteira na 4b, porque não havia falta resolvida; **a 4c a tornou preguiçosa** (só a página dos argumentos nasce mapeada) | Decisão 5 e o desenho de pilha; o `mmap_base` fica para a 4c |
| Capacidade da árvore | 32 regiões por processo (`REGION_CAPACITY`, 1 KiB de heap) | No máximo 16 `PT_LOAD` mais a pilha, com folga para a 4c; [DEBT-0075](../tech-debt/0075-vmar-com-capacidade-fixa-de-regioes.md) |
| Teto de `p_memsz` | `EXEC_MAX_LOADS_BYTES = 64 MiB` em `hal::abi` | O "ex.: 64 MiB" da tabela de validação |
| Auxv e pilha inicial em `hal::abi` | O kernel (`InitialStack::build`) e o `user/rt` (`Env::from_stack`) partilham as tags, e o teste do `rt` lê o que o `build` monta | Um lado só não deriva do outro |
| `debug_write` por linha | `println!` formata a linha num buffer de 512 bytes e faz uma `debug_write`; o kernel põe `[user] ` em cada chamada | Peça por peça viraria uma linha por pedaço |
| `rt` testado no host | `make test` roda `cargo test --package rustos-user-rt` (o auxv, o `println!` e o alocador); `default-members` segue sem `user/` e sem `arch/*-user` | Os crates só compilam o asm para o alvo bare-metal; o `make clippy` os confere nos dois alvos |
| Os casos de falha da 4a | Ficam no blob de `global_asm!` do `arch/<isa>`, atrás de `selftest`; só o `big` é programa em `user/tests/` ([DEBT-0086](../tech-debt/0086-casos-de-falha-de-usuario-no-blob-do-kernel.md)) | Cada caso pede `asm!`, que o usuário em Rust seguro não tem |
| `rdx` na entrada | O `enter_user` do x86_64 zera todo registrador menos `rdi` e `rsi` desde a 4a; a 4b o torna contrato (o caso `hello` do blob confere `rdx == 0` na primeira instrução) | O psABI o lê como ponteiro de `atexit`: uma libc o registraria |
| `#[used]` | O `user/rt` não usa `#[used]` ([DEBT-0074](../tech-debt/0074-elf-recusa-osabi-gnu-do-used.md)); o `init` sai com `EI_OSABI = 0` | — |

**Conflito do `x0` com a glibc (para a fase 5 decidir).** A ABI põe o handle de boot em `rdi`/`x0`. No x86_64 a
glibc lê `rdx` como `atexit` e o RustOS o zera; no aarch64 a glibc lê o `x0` do mesmo jeito (`rtld_fini`), e o RustOS ali põe o
handle. Uma libc glibc rodando como programa nativo registraria o handle como função de saída. O musl ignora o `x0` e só lê `argc` e
`argv`. A 4b entrega `0` e não conflita; quando o handle for diferente de zero (4c), a fase 5 escolhe: outro registrador no aarch64
(por exemplo `x1`), ou o adaptador Linux zerar o `x0` antes de entregar o controle a uma glibc. Fonte: a pesquisa de 2026-11-18
sobre a pilha inicial (`sysdeps/aarch64/start.S` da glibc e `crt1.c` do musl, **secundária**; o layout de `envp` e do auxv no aarch64
fora do Linux segue **NÃO VERIFICADO**).

**O que o autoteste confere** (`make smoke`, modo de teste, nas duas ISAs), além dos casos da 4a: o `init` carrega (`init loaded (N
segments)`: 3 no x86_64, 2 no aarch64 na 4b e 3 nas duas desde a parte 1 da 4c), roda, imprime `[user] init: ok (...)` e sai com 0 (`pid 1 exited (code 0`); o `big` sai com 0 e os
frames voltam **exatamente** como depois do `init` (a folga é medida uma vez, depois de uma rodada de aquecimento, com o processo preso à CPU de
quem testa, para o frame devolvido cair no cache que `free_frames` conta); o carregador recusa o arquivo cortado na tabela de program
headers, o de outra máquina, o sem magic e o vazio, e, com um gancho de teste que acaba com o frame na página 1, 3, 5 ou 18, devolve cada frame
numa falha **tardia** (depois de segmentos e de parte da pilha mapeados); e um ELF sintético com `p_vaddr = 0x40_3010`, `p_offset = 0x2010`,
`p_filesz = 0x20` e `p_memsz = 0x2020` num arquivo cheio de `0xAA` carrega com o começo da página e a cauda `.bss` zerados (lido pelos frames
do espaço, sem rodar: um programa em `forbid(unsafe_code)` não lê antes do próprio `p_vaddr`). O teste exato achou um vazamento da 4a: o
`Box<Process>` nunca era solto, porque `terminate` não volta; agora `terminate` solta a árvore de regiões antes de encerrar a tarefa. O
`user/rt` não deixa o programa forjar ponteiro: `Env` tem campos privados e a contagem de `PT_LOAD` sai de `from_stack`, que é `unsafe`.

## A fatia 4c, parte 1, como entrou

**Descritivo.** O que a parte 1 da 4c (PR `feat/phase4c-handles`) tornou verdade, nas duas ISAs: o
`core/object` ligado ao kernel, as chamadas de handle, canal e espera, `debug_write` por handle e o canal
de boot. VMO, VMAR, `process_create`, `thread_create` e `process_kill` são as partes seguintes (o
`thread_create` e o `process_kill` seguem `NotSupported`). A paginação sob demanda, a página zero e o
*shootdown* de TLB entraram em outra PR ([seção abaixo](#a-fatia-4c-paginação-sob-demanda-e-shootdown-como-entrou)). Os números estão em
[SDD 07](07_desempenho_e_recursos.md#handles-canais-e-espera-no-kernel-fatia-4c-parte-1).

| Peça | Onde mora |
| --- | --- |
| `Obj`, o enum fechado de `Arc` (`Process`, `Channel`, `Console`; `Vmo`, `Vmar` e `Thread` entram com as chamadas que os criam), `Lock` sobre o `IrqLock`, `VecSlots` (`try_reserve`, e o clamp do índice do handle por `platform::array_index_nospec`), `Work` (a lista de trabalho), o par de pontas, a cobrança de pontas por processo | `kernel/src/object.rs` |
| `Process` com a tabela de handles, o objeto `ProcessObj` e a cobrança; `with_current`; a morte esvazia a tabela (`drain`) antes de liberar o espaço e levanta `TERMINATED` depois | `kernel/src/process.rs` |
| O canal de boot e o console de um programa plano de teste | `kernel/src/process/boot.rs` |
| `handle_close`, `handle_duplicate`, `handle_replace`, `object_wait_one`, `channel_create`, `channel_write`, `channel_read`, `debug_write(console, ptr, len)`; a tabela de despacho ganhou as entradas | `kernel/src/syscall.rs`, `kernel/src/syscall/handles.rs` |
| `block_until`, `wake_waiters`, `wait_bit`, `clear_permit`, (a ponte com o escalonador) | `kernel/src/task.rs` |
| `ReadActual`, `encode_boot_message`, `parse_boot_message`, `MAX_CHANNEL_ENDS`, `USER_HANDLE_ARRAY_MAX` | `core/hal/src/abi.rs` |
| Syscalls seguras, `boot::{read, attach_console}`, `set_console` (o `println!` escreve pelo console por handle) e a feature `raw-syscalls` (`raw::syscall` é `unsafe fn`, só dos `user/tests/*`; o xtask compila cada programa numa invocação própria e o `check-layers` confere) | `user/rt` |
| O `init` (anexa o console antes de qualquer conferência; sem ele sai com 99) e o programa de teste `handles` (casos hostis, medida e espera bloqueante com o kernel do outro lado) | `user/init`, `user/tests/handles` |

**A espera.** `object_wait_one` bloqueia a tarefa (`block_until`: `Blocked` sem prazo, `Sleeping` com prazo) e quem
muda o sinal a acorda. O objeto guarda uma máscara de 64 bits com **um bit por slot de tarefa** (`Waiters`; um
`const _` confere `MAX_TASKS <= 64`): registrar é um `fetch_or`, acordar é `swap(0)` e `wake_waiters`, sem alocar e
com qualquer lock preso. O par de pontas compara os sinais das duas pontas antes e depois de cada operação sob o lock
dele e acorda só a ponta cujo sinal mudou. A ordem é registrar, **reconferir o sinal**, bloquear: um aviso que chega
entre a conferência e o bloqueio fica como *permit* da tarefa (`wake_task` com `Wake::Signal` o deixa em quem está
`Running` ou `Ready`) e `block_until` o consome, então não se perde acordar. A chamada volta sem motivo certo e
reavalia em laço, por isso o acordar falso não aparece ao usuário. O despacho segue com a IRQ mascarada
([DEBT-0081](../tech-debt/0081-debug-write-de-512-bytes-com-a-irq-mascarada.md)): a tarefa bloqueada cede a CPU como
`sleep_until` faz, e a CPU roda as outras com a IRQ ligada.

**O canal de boot, e como a mensagem fica até a parte 3.** Todo programa carregado do initramfs
nasce com a ponta B de um canal na tabela, no handle `1`, e recebe esse valor em `rdi`/`x0`. O kernel monta a
mensagem por uma tabela de handles própria dele (a mesma `channel_write` do usuário), escreve e **fecha a ponta A**:
o programa lê a mensagem e vê `PEER_CLOSED`. A mensagem de boot é a do contrato (`magic`, `version`, `count`, `reserved`
e um `u32` de `kind` por item), com `count = 2` e `kind`s `[1 Process, 3 Console]`, na ordem dos handles da mensagem:
o Process do próprio programa (todos os direitos de processo) e o console (`WRITE`, `DUPLICATE`, `TRANSFER`). O
VMAR raiz (`kind` 2) e o VMO do initramfs (`kind` 4) **entram na parte seguinte**, quando existirem como objetos; o
leitor casa por `kind` e não por posição, então a mensagem maior não quebra o `init`. O programa plano de teste
(`Source::Flat`) não tem canal de boot: recebe só o console, no handle `1`.

**Divergências e preenchimentos do desenho** (decisões do agente; nenhuma muda número, `Status` nem o layout da mensagem
de boot):

| Decisão | O que ficou | Por quê |
| --- | --- | --- |
| `actual` do `channel_read` | `actual*` aponta para dois `u64` seguidos, `{ bytes, handles }` (`hal::abi::ReadActual`), como o `out` do `channel_create`. É gravado **antes** de a mensagem sair da fila (a falha da gravação é `OutOfRange` e a mensagem fica) e também em `BufferTooSmall`, com o que a mensagem pede. O contrato só dizia "tamanhos em `actual`" | Preenche o que o contrato numérico deixava aberto; **a confirmar pelo dono**, é parte do que o mundo de fora enxerga |
| Sinais desconhecidos | `signals` com bit além dos quatro definidos é `InvalidArgs`, como `signals == 0` | Bit reservado é `InvalidArgs` em toda a ABI; deixa o bit livre para sinal novo |
| `debug_write` sem console | Handle que não é de console é `WrongType`; sem `WRITE`, `AccessDenied`; o handle é conferido antes de `len` | A ordem das outras chamadas: o handle que autoriza primeiro |
| A morte da ponta esvazia a fila fora do lock | Ver [Ordem de lock](#ordem-de-lock-lacuna-l3) e a decisão "Lista de trabalho" | Cumpre o requisito de não alocar com o lock do par preso sem pagar 6 KiB por par numa lista reservada |
| O par de pontas nasce por cópia de um modelo | `EMPTY_PAIR` (7 KiB na imagem, `rodata`) é copiado para o `Arc` com `copy_nonoverlapping` e o número do par é dado depois (`Channel::assign_id`). Construir o `Channel` na pilha (7 256 bytes, mais as cópias do `Arc::new`) estourava a pilha de 16 KiB: o `init` morria em `#DF` | Dois `unsafe` com `// SAFETY:` em `kernel/`; sai quando a fila deixar de ser inline ([DEBT-0091](../tech-debt/0091-fila-do-canal-com-armazenamento-inline.md)) |
| Cobrança de pontas | `MAX_CHANNEL_ENDS = 256` (`hal::abi`); cada ponta guarda uma `Charge` do criador e a devolve no `Drop`. O canal de boot do kernel não é cobrado | A defesa que o SDD escolheu para o vazamento entre filas ([DEBT-0090](../tech-debt/0090-ciclos-entre-filas-de-canais-nao-coletados.md)) |
| Teto de 64 handles no array de usuário | `USER_HANDLE_ARRAY_MAX = 64`; acima disso, `InvalidArgs` sem consumir (o kernel não leu quais são). Entre 9 e 64, `InvalidArgs` **consumindo** (a regra do canal) | O teto do Zircon, como o SDD já dizia |
| O erro depois de ler o array consome | Bytes acima do limite, ponteiro de bytes ruim e canal inválido consomem o array; só `ShouldWait` e a falha ao **ler** o array não | A regra geral para quem move handle |
| Alocação de objeto e de mensagem | O [guidelines 05](../guidelines/05_recursos_e_desempenho.md) deixa a syscall que cria objeto ou handle, ou carrega mensagem, alocar **com `try_*`** (a falta é `NoMemory`). A parte 1 só cumpre nos slots da tabela: falta de heap num `Arc` ou na cópia dos bytes ainda aborta (`handle_alloc_error`) ([DEBT-0142](../tech-debt/0142-objeto-do-kernel-sem-try-new.md); o teto em bytes entrou na parte 3) | `Arc` não tem alocação fracassável na toolchain estável |
| `init` no boot normal | Segue só no modo de teste ([DEBT-0087](../tech-debt/0087-init-so-no-boot-de-teste.md)) | Na parte 3 o `init` passou a criar um filho; o orçamento do boot é do dono |

**O que o autoteste confere** (`make smoke`, modo de teste, nas duas ISAs), além dos casos anteriores: o `init`
lê a mensagem de boot (`process` e `console`), vê `PEER_CLOSED` na ponta do kernel e nenhuma segunda mensagem,
escreve pelo console, **recusa** duplicar o console com `READ`, cria um canal, vê `TimedOut` com a fila vazia,
escreve 64 bytes, espera `READABLE`, lê, transfere um handle e o recebe de volta, fecha a ponta e vê `PEER_CLOSED`
(`[user] init: boot channel, console and handles ok`). O `init` anexa o console antes de qualquer conferência, então falha de conferência e pânico saem no log; se o anexo falha, sai com 99. O programa `handles` (`[user] handles: ok (N assertions)`, 181 hoje: `expect` e `status` do programa, não chamadas):
handle forjado de seis formas em cada chamada, handle velho (fechado duas vezes, e o valor segue morto com o slot
reusado), sem direito (`DUPLICATE`, `WRITE`, `READ`, `WAIT`), de outro tipo, `replace` com direito maior (o velho
morre), bits de direito reservados, a própria ponta e o mesmo handle duas vezes no array, handle sem `TRANSFER`,
array com ponteiro ruim (nulo, faixa nula, não mapeado, do kernel), mais de 64 e 9 handles, bytes acima do limite
e contagem absurda, leitura que não consegue copiar (a mensagem e o handle ficam na fila, inteiros, depois de cada
falha), `BufferTooSmall`, fila cheia (`ShouldWait` não consome), `PeerClosed`, os 256 pontas da cobrança e o
`NoMemory` da 257ª, os 4 096 handles da tabela (que a morte do processo fecha), `object_wait_one` com prazo vencido,
com prazo no futuro (espera até o prazo), sem sinal e com bit desconhecido, e a espera **bloqueante**: o kernel guarda
a outra ponta do canal de boot, o programa bloqueia com prazo de 10 s e só volta quando o kernel escreve (depois de
20 ms) e, na segunda espera, quando o kernel fecha a ponta; o kernel confere que tudo levou menos de 5 s.

## A fatia 4c, paginação sob demanda e *shootdown*, como entrou

**Descritivo.** O que a parte da 4c que não mexe em handle nem em syscall (PR `feat/phase4c-demand-paging`) tornou verdade, nas duas ISAs: a falta de página de usuário resolvida, a página zero compartilhada, a reserva de páginas e o *shootdown* de TLB. O que o SDD já escrevia em [Página zero compartilhada e COW](#página-zero-compartilhada-e-cow) e em [*Shootdown* de TLB](#shootdown-de-tlb-paga-a-dívida-0055-na-4c) vale como estava; esta seção diz onde mora e o que mudou. Os números estão em [SDD 07](07_desempenho_e_recursos.md#paginação-sob-demanda-e-shootdown-fatia-4c).

| Peça | Onde mora |
| --- | --- |
| Região preguiçosa (`Attrs::lazy`), a decisão da falta (`Vmar::plan_fault`: `Mapped`, `FaultPlan`, `FaultError::Unbacked`) | `core/mm/src/vmar.rs`, com teste de cada caso |
| Folha de 4 KiB lida, trocada e removida (`PageTables::leaf`, `remap_page`, `unmap_page`, `forget`); `MmuControl::entry_removed`; `MapError::NotMapped` | `core/mm/src/tables.rs`, `core/hal/src/paging.rs` |
| `Handlers.copy_fault`; o `fault` de origem de usuário volta se o núcleo resolveu | `core/hal/src/interrupt.rs`, `arch/{x86_64,aarch64}/src/{traps,exception}.rs` |
| Vetor `0xFC` do IPI de invalidação, `X86Ipi::send_shootdown`, `flush_pages`; `entry_removed` com `invlpg` (x86_64) e `tlbi vaale1is` (aarch64) | `arch/x86_64/src/{lapic,apic,mmu}.rs`, `arch/aarch64/src/mmu.rs` |
| A página zero (criada no primeiro uso), a resolução da falta, o `unmap_page`, a espera por CPU ativa, a promessa de páginas | `kernel/src/uspace.rs`, `kernel/src/commit.rs` |
| Caixas e IPI de *shootdown* | `kernel/src/tlb.rs` |
| Máscara de CPUs que rodam o espaço (`active`), mantida pela troca de tarefa | `kernel/src/task.rs` (`mark_active`, `bind_user`, `unbind_user`), `kernel/src/uspace.rs` |
| Pilha preguiçosa e `.bss` preguiçoso | `kernel/src/loader.rs` |
| A falta de página da tarefa corrente | `kernel/src/process.rs` (`resolve_user_fault`), `kernel/src/interrupts.rs` |
| O programa de teste, o autoteste e as linhas do smoke | `user/tests/lazy`, `kernel/src/process/selftest/demand.rs`, `xtask/src/qemu.rs` |

**O que a falta faz** (`UserSpace::resolve_fault`, o mesmo código nas duas ISAs):

| Situação | Resultado |
| --- | --- |
| Endereço fora de região, na guarda, ou acesso que a região não permite | O processo morre, como na 4a (`FaultError::{NoRegion, Guard, Permission}`) |
| Página ausente numa região **não** preguiçosa (imagem, `.data`) | O processo morre (`Unbacked`): o carregador mapeia tudo antes de rodar |
| Leitura de página ausente, região preguiçosa | Mapeia a página zero (só leitura, sem execução, de usuário) |
| Escrita em página ausente, região preguiçosa | Frame novo e zerado, gravável |
| Escrita sobre a página zero | Frame novo e zerado; a folha é trocada (**apaga, invalida, grava**) e as outras CPUs do espaço são invalidadas e esperadas |
| Página que já serve ao acesso (outra CPU a resolveu, ou o TLB estava velho) | `Retry`: esquece a tradução desta CPU e repete a instrução |
| Execução numa região preguiçosa | O processo morre: página zero e frame novo nunca ganham execução |
| Sem frame para a página nem para as tabelas | O processo morre com a falta de página, e a linha da morte diz `(out of memory)`; na cópia de uma syscall o erro é `NoMemory` em vez de `OutOfRange`. A promessa cobre as páginas preguiçosas e as tabelas delas, mas não o que o kernel aloca depois nem as tabelas das páginas que já nasceram mapeadas: [DEBT-0121](../tech-debt/0121-reserva-so-das-paginas-preguicosas.md). Medido: pedir tantas páginas quanto há frames livres é recusado (o `.bss` de 16 000 páginas com 16 100 frames livres, que a primeira versão deixava passar, falha no `spawn`) |

A cópia usuário↔kernel resolve do mesmo jeito (`Handlers.copy_fault`): um `debug_write` sobre página nunca tocada lê zero em vez de devolver `OutOfRange`.

**Regiões preguiçosas.** A pilha (15 das 16 páginas: a última, com `argc`, `argv`, `envp` e o auxv, nasce mapeada) e as páginas de um segmento `PT_LOAD` **gravável** que só têm `.bss` (depois da última página com bytes do arquivo). O resto dos `PT_LOAD` segue como na 4b: cópia na carga. Um segmento com `.bss` não ganha região à parte: a região inteira vale `lazy`, e as páginas com arquivo estão sempre mapeadas, então a falta nelas é só `Retry`. Segmento não gravável fica inteiro mapeado.

**A página zero.** Um frame global, zerado, criado na primeira leitura de página nunca tocada (não no boot: o boot normal não paga 4 KiB nem o zerar) e nunca devolvido. Nenhuma contagem de referência. `destroy` o pula ao liberar as folhas. O autoteste percorre as tabelas de um espaço e confere que toda folha da metade de usuário que o mapeia é só leitura, sem execução e de usuário.

**A promessa de páginas** ([decisão 9](#decisão-9--overcommit-na-abi-nativa), a parte que cabe aqui). `commit::reserve` promete as páginas preguiçosas de um processo na criação, **mais as tabelas de página que elas pedem** (uma por 512 páginas, e mais uma), contra os frames livres menos o que já foi prometido e ainda não ganhou frame; o carregador promete **depois** de mapear o que nasce com frame, para os livres refletirem o uso. Passa, ou o `spawn` falha com `OutOfFrames`. A página que ganha frame próprio devolve 1 da promessa, e a morte devolve o resto. O que falta (`NoMemory` em `vmo_create` e `vmar_map`, o modo por processo, contar o que o kernel aloca) é a [DEBT-0121](../tech-debt/0121-reserva-so-das-paginas-preguicosas.md) e entra com os VMOs.

**O *shootdown*.**

| | x86_64 | aarch64 |
| --- | --- | --- |
| Remover ou trocar a folha | `invlpg` local; depois `tlb::shoot_page` | `dsb ishst`, `tlbi vaale1is`, `dsb ish`, `isb`: alcança todas as CPUs sem IPI (só a última folha, de todos os ASIDs: a 4c usa o ASID 0 em tudo) |
| Quem recebe | Vetor `0xFC`. Uma caixa por par (destino, origem), 8 x 8 x 24 bytes (1,5 KiB de `.bss`), com a faixa e um bit `pending`; o destino invalida e baixa o bit | — |
| Espera | O iniciador manda o IPI a cada CPU de `active` (menos ele e as offline) e gira até todos os bits baixarem, **atendendo as caixas que chegam a ele** enquanto gira: duas CPUs que se invalidam com a IRQ mascarada não se travam. Sem resposta em 5 s o kernel entra em pânico | A espera é o `dsb ish` |
| Faixa | Até 32 páginas, `invlpg` uma a uma; acima disso, reescreve o `CR3` | — |
| Ordem da máscara | A troca de tarefa **sobe** o bit da CPU antes de a raiz entrar nela e **baixa** depois de a raiz sair; quem invalida grava a entrada, faz `fence(SeqCst)` (`UserSpace::flush_remote`) e só então lê a máscara: o store da entrada e a leitura precisam de StoreLoad, que a ordem `SeqCst` só da leitura não dá (o Linux sobe o `tlb_gen` antes de ler o `mm_cpumask` em `flush_tlb_mm_range`, `arch/x86/mm/tlb.c`). O bit sobe com `SeqCst` | idem |

**A morte do processo** espera `active == 0` (`UserSpace::wait_inactive`, chamado por `terminate`) antes de liberar as tabelas; `destroy` só confere (`assert`) e vale para espaço que ninguém ligou ou que já esperou, porque esperar depois de mover o espaço olharia uma cópia. `tlb::service` mascara a IRQ ao entrar: o laço de espera não apaga um pedido novo que chegou entre a leitura e a baixa do bit. Com uma thread por processo a espera nunca gira: a thread já saiu da raiz no passo 3 da [ordem da morte](#falhas-de-usuário). O espaço **não pode se mover** enquanto alguma CPU o roda: a máscara que a tarefa guarda é o endereço do campo dele, e `terminate` espera antes de tirá-lo da `Box`. O parar de thread em outra CPU, com `thread_create`, é da fatia que o criar; a espera já está no lugar e entra em pânico se achar CPU ativa.

**Decisões do agente** (nenhuma muda contrato nem regra):

| Decisão | O que ficou | Por quê |
| --- | --- | --- |
| Trocar a folha apagando, invalidando e gravando, nas duas ISAs | `remap_page` | O aarch64 pede *break-before-make* para trocar o endereço de saída de uma folha válida (o `ptep_clear_flush` do Linux em `wp_page_copy`); no x86_64 custa uma escrita |
| `fault` volta quando `from_user` e o núcleo resolveu | O `on_fault` só devolve se resolveu; senão `user_fault` não volta | O arch não precisa saber do resultado, e um erro do núcleo vira laço de falta, não memória alheia |
| `copy_fault` à parte do `fault` | Um gancho novo em `Handlers` que diz se a cópia se repete | A falta da cópia chega do kernel (`from_user` falso) e o arch já tinha o ponto de recuperação dela |
| A página zero nasce no primeiro uso | `ZERO_FRAME` por `compare_exchange` | O boot normal não a usa, e duas CPUs que a criam juntas ficam com uma só |
| `.bss` preguiçoso só em segmento gravável | `eager = ceil((lead + filesz) / 4096)` páginas | Uma página que mistura arquivo e `.bss` já tem o arquivo; segmento só de leitura com cauda zero é raro |
| Sem lock por espaço na falta | Uma thread por processo | Paga na parte 3, com `thread_create`: um lock por espaço ([abaixo](#a-fatia-4c-parte-3-como-entrou)) |
| O controle do teste de *shootdown* é só informação | Sem a parte remota a tradução velha sobrevive: **sim** no x86_64 (TCG) e no aarch64 em TCG, **não** no aarch64 em HVF | Medido: no HVF o controle não tem dentes (0 leituras velhas em 240 000) e em TCG tem (26 553 em 120 000). O gate do aarch64 só distingue `is` de local em TCG, que o último passo do `make pr` cobre (`RUSTOS_ACCEL=tcg RUSTOS_CPU=a64fx`); em HVF ele só prova que nada velho aparece |

**O que o autoteste confere** (`make smoke`, modo de teste, nas duas ISAs; linha `user self-test:` com `fault resolver`, `lazy program` e `tlb shootdown`):

- O resolvedor, num espaço montado à mão: leitura (página zero compartilhada), leitura de novo (`Retry`), escrita sobre ela (cópia, e a página segue zero), escrita em página ausente (frame novo), guarda de baixo e de cima, buraco, escrita em página de leitura, execução; a promessa sobe e volta, e rodado duas vezes devolve **exatamente** os frames; uma promessa maior que a memória é recusada.
- O `lazy` (`user/tests/lazy`) desce a pilha por nove páginas que ninguém mapeou, lê antes de escrever um `.bss` de 32 MiB (página zero), escreve sobre ela e no `.bss` virgem, e confere que o que não foi escrito segue zero (`[user] lazy: ok (...)`); o kernel confere que passaram pela falta pelo menos 4 094 páginas de cada caminho, que o processo saiu com 0, que os frames voltaram como depois do `init` e que a promessa zerou (`[rustos] user: demand paging: ...`); as páginas com frame próprio ao sair passam de `2 x 4 096 + 6` (as duas matrizes e a pilha), e o programa confere vários bytes de cada página depois da cópia. A página zero é relida inteira depois dos casos do resolvedor e segue zero.
- O *shootdown* entre CPUs (`[rustos] tlb shootdown: ok (cpu A -> cpu B: ...)`): a CPU B roda uma tarefa com o espaço carregado, lê uma página lida por ela (página zero) e uma página própria; a CPU A troca a primeira por um frame novo com dado (a cópia da página zero) e remove a segunda. Depois disso a leitora vê o dado novo e a falta, nunca a tradução velha. O mesmo roteiro com a parte remota desligada é o controle (a tradução velha sobrevive no x86_64 e no aarch64 em TCG). O dono do espaço espera a leitora sair antes do `destroy`.
- O estresse em anel (`[rustos] tlb shootdown ring: ok (cpus N, ...)`), com todas as CPUs online: a CPU i dirige o espaço da CPU i+1 (página zero, cópia com dado novo, remoção, em laço) e vigia o seu, que a CPU i-1 dirige, com a IRQ aberta e depois mascarada como na falta. Cada leitura vê o valor do passo, nunca o velho: 300 rodadas por passo, 3 600 leituras conferidas. Sem a parte remota o mesmo anel vê 1 446 de 3 600 leituras velhas (x86_64, TCG). Veio do revisor independente, que o rodou com 4 e 8 CPUs em 25 boots por configuração (0 falhas; sem a parte remota, de 1 000 a 26 000 leituras velhas). O anel reserva 1 página a cada cópia porque a região dele segue na árvore depois de `unmap_page`.
- O `init` imprime os frames que consome ao morrer (`init at exit: 5 data, 7 tables, 0 of 15 lazy pages backed`).

## O M1: programas pelo shell, como entrou

**Descritivo.** O M1 é o marco "minimamente usável" do [roadmap](08_roadmap.md): subir o sistema no **boot normal** e, no
shell que já existe, listar os programas do initramfs, rodar um como **processo isolado**, vê-lo no `ps` e ver um programa
que falha morrer sozinho sem derrubar o sistema. Nenhuma syscall, formato de executável nem contrato de fora mudou
(nenhum dos casos de parada do `AGENTS.md`): o shell usa o carregador da 4b e o canal de boot da 4c, parte 1, como o `init`.

| Peça | Onde mora |
| --- | --- |
| Os comandos `ls`, `run` e a coluna `pid` do `ps`, e a porta (`Host::programs`, `Host::run_program`, `RunOutcome`, `KillCause`, `TaskInfo::pid`), puros e testados no host com um `Host` falso | `core/shell/src/{commands,host}.rs` ([SDD 16](16_terminal.md#comandos-que-falam-com-processos-m1)) |
| Ler o `cpio` para o `ls`; carregar com `process::spawn`; esperar o fim do processo | `kernel/src/programs.rs` |
| O fim do processo: `terminate` chama `programs::finished(pid, evento, sem_memória)`, que guarda o resultado numa tabela de 16 posições e acorda quem espera ([DEBT-0133](../tech-debt/0133-fim-de-processo-numa-tabela-de-16-posicoes.md)) | `kernel/src/process.rs` (uma linha) e `programs.rs` |
| O número do processo no `ps`: `Slot::pid` da tarefa, posto por `task::set_pid` na entrada do processo e zerado em `unbind_user` | `kernel/src/task.rs` |
| `hello`, `crash`, `sysinfo` e `count`, `forbid(unsafe_code)` sobre o `user/rt`, na imagem de produção ao lado do `init`; os de teste (`big`, `handles`, `lazy`) seguem só na imagem com `selftest` | `user/{hello,crash,sysinfo,count}`, `xtask/src/user.rs` |
| A sessão que o smoke digita (`ls`, `run hello`, `run sysinfo`, `run crash`, `run nothere`, `run count &`, `ps`) e confere, nos dois modos | `xtask/src/qemu.rs` (`SESSION`) |

**O que o `run` faz.** `System::run_program` acha o arquivo no `cpio`, chama `process::spawn(Source::Elf)` e, sem `&`,
dorme (`block_until` sem prazo, registrado em `Waiters`) até `terminate` acordá-la. O processo é o mesmo do `init`: espaço
próprio, pilha preguiçosa com guarda, canal de boot com `process` e `console`, e a saída `[user] …` pelo console por handle.
O shell mostra `exit code N (pid P)` ou `killed: <causa> (pid P)`, cada resposta numa **única escrita** (a saída de um processo em segundo plano não a rasga), com a causa tirada do evento de falha (falta de
página com o endereço **de usuário** e o tipo do acesso, instrução ilegal, acesso desalinhado, falha aritmética, `out of
memory` quando a falta de página não achou frame) e nunca um endereço do kernel (o gate de vazamento do smoke confere a
sessão). A tarefa do processo se chama `user` no `ps` ([DEBT-0134](../tech-debt/0134-ps-mostra-user-como-nome-de-tarefa-de-processo.md)).

**Decisões do agente** (nenhuma toca os seis casos de parada):

| Decisão | O que ficou | Por quê |
| --- | --- | --- |
| A porta é do `core/shell` | `Host::programs` e `Host::run_program` com tipos próprios (`RunOutcome`, `KillCause`); nenhuma porta nova no `hal` | O `core/shell` já é a porta de tudo o que o shell pede; um tipo do `hal` pediria um terceiro crate para dois consumidores |
| `run <programa> &` cabe | Sem esperar, o `run` devolve `started pid P` e o `ps` mostra o processo enquanto ele roda (`count` dura 2,5 s). O resultado de quem termina fica só no log do kernel (`[rustos] user: pid N exited …`) | A espera é a mesma tabela de fins; o que o `&` poupa é só a espera. A saída dele cola no prompt ([DEBT-0132](../tech-debt/0132-saida-de-segundo-plano-cola-no-prompt.md)) |
| Argumentos não passam | `run hello x` é recusado no shell | `argv` mexe no `InitialStack` do `core/hal` e no `process_create` da parte 3 ([DEBT-0130](../tech-debt/0130-run-sem-argumentos-para-o-programa.md)) |
| Não há Ctrl-C | O `run` em primeiro plano espera sem prazo; o kernel não mata processo de fora | Depende de `process_kill` e da morte de thread bloqueada ([DEBT-0131](../tech-debt/0131-sem-ctrl-c-nem-kill-de-processo.md)). Todo programa de demonstração termina sozinho |
| O `init` não sobe no boot | O boot normal não lança processo nenhum; `run init` roda as conferências dele | O shell lança o que o usuário pede; o `init` como primeiro processo fica com a parte 3 ([DEBT-0087](../tech-debt/0087-init-so-no-boot-de-teste.md)) |
| `crash` é Rust seguro | Desce a pilha por recursão até a página de guarda; a falta de página não se resolve | O `rt` não tem leitura de endereço arbitrário em função segura (seria um `unsafe` que só o `rt` pode ter), e a guarda é um endereço proibido de verdade. Exercita também a paginação sob demanda |
| Linhas do ciclo de vida só na serial | `entered ring3/el0`, `pid N exited (…)`, `stopped by …` e `fault contained` saem por `kprintln_serial!` (`console::print_serial`): serial sim, tela não. As de `[user]` (a saída do programa) seguem nas duas | O [guidelines 05](../guidelines/05_recursos_e_desempenho.md) manda o boot normal imprimir só o que um gate confere ou o que avisa, e a tela da imagem de produção mostra o que o shell já diz (`exit code`, `killed: …`). A serial é o diagnóstico do kernel e o smoke lê só ela, então o modo de teste não perde nada |
| `terminate` solta os donos antes de sair | O `Box<Process>`, o `Arc<ProcessObj>` e a cota ficam num bloco interno que devolve `(pid, no_memory, cpu)`; `task::exit()` é `-> !` e **nunca roda o `Drop`** de local vivo | Sem o bloco, cada processo vazava cerca de 290 bytes de heap do kernel (200 `run hello`: 16,7 para 73,8 KiB). O smoke agora roda `mem`, 20 `run hello` e `mem` e exige o heap em uso igual; sem o bloco ele reprova (17,8 para 23,4 KiB no aarch64) |
| O pid só existe para quem existiu | `spawn_keeping` devolve o número ao contador se a tarefa não nasce (`NEXT_PID.compare_exchange`) | A tabela de tarefas cheia (`TaskError::NoSlot`) gastava um pid; agora `run` responde `too many processes` e o número volta |
| Falta no endereço nulo, baixo ou do kernel | O evento do processo (ABI) não muda: endereço de kernel segue virando 0. O kernel passa a `finished` e a `log_fault` um `kernel_half` à parte; o shell diz `(null or low address)` abaixo de `USER_MIN` e `a kernel address`, sem endereço, na metade do kernel | Antes as duas saíam como "outside the user half" |
| Resultado por pid numa tabela, não `LAST_EVENT` | `LAST_EVENT` é um só, serve ao modo de teste; a tabela de 16 serve ao `run &` com vários processos | [DEBT-0133](../tech-debt/0133-fim-de-processo-numa-tabela-de-16-posicoes.md) |
| `process.rs` só ganha | Uma linha em `terminate`, o `pid` em `process_main` e o `allow(dead_code)` do módulo sai (o de `Spawned`, `last_event` e `BootPeer` passa a ser por item, `cfg_attr(not(selftest))`) | A parte 3 da 4c reescreve `process_create` no mesmo arquivo |

**O que o smoke confere.** Depois do `boot ok` e do prompt, nos dois modos e nas duas ISAs, o `xtask` digita a sessão e
confere cada resposta: o `ls` lista as cinco entradas (`init`, `hello`, `crash`, `sysinfo`, `count`); `run hello` e
`run sysinfo` imprimem a linha `[user]` e saem com `exit code 0`; `run crash` mostra `killed: page fault` e o kernel
`user: fault contained`; um programa que não existe é recusado; `run count &` volta na hora e o `ps` mostra a linha
`user` com o pid; a última linha do `count` (`[user] count: done`) chega sozinha; e o heap do kernel em uso, lido no `mem` antes e depois de 20 `run hello`, é o mesmo (cada processo devolve tudo ao morrer). No boot normal as linhas do modo de teste
seguem proibidas no log **inteiro**, boot e sessão, menos as três que lançar um processo produz (`entered ring3/el0`, `pid 1 exited (code 0,` e `fault contained`; `init loaded` e `user self-test:` continuam proibidas).

**Números** (bytes e `boot time:`) estão no [SDD 07](07_desempenho_e_recursos.md#o-carregador-na-imagem-enxuta-m1).

## A fatia 4c, parte 3, como entrou

**Descritivo.** O que a parte 3 da 4c (PR `feat/phase4c-processes`) tornou verdade, nas duas ISAs: VMO e VMAR como objetos, `process_create`, `thread_create`, `thread_exit` e `process_kill`, o teto em bytes por processo, o canal de boot completo e o `init` criando um filho. A chamada que o texto do desenho chama de `process_start` é a primeira `thread_create`: o contrato numérico não mudou (a ABI tem 19 chamadas, todas atendidas). Os números estão em [SDD 07](07_desempenho_e_recursos.md#processos-vmo-e-vmar-fatia-4c-parte-3).

| Peça | Onde mora |
| --- | --- |
| VMO anônimo (preguiçoso, dono dos frames) e físico (initramfs, só leitura, sem zerar nem devolver) | `kernel/src/vmo.rs` |
| A memória do processo (`Mem`: espaço, árvore de regiões, os VMOs mapeados) sob um `IrqLock` que espera o *shootdown* girando; `map`, `unmap`, a falta | `kernel/src/process/mem.rs`, `kernel/src/sync.rs` (`lock_serving`) |
| `Process` (objeto, tabela, cota, vida e morte), `ThreadObj`, `VmarObj`, `start_thread`, `leave_current` | `kernel/src/process.rs` |
| `vmo_create`, `vmar_map`, `vmar_unmap`; `process_create`, `thread_create`, `process_kill` | `kernel/src/syscall/{memory,tasks}.rs`; `thread_exit` e `process_exit` em `syscall.rs` |
| `Obj::{Thread,Vmo,Vmar}`, `Lifecycle` (`TERMINATED`), `Quota`, `ByteCharge`, `Bytes` (mensagem cobrada) | `kernel/src/object.rs` |
| `FaultPlan::MapObject` | `core/mm/src/vmar.rs` |
| `unmap_pages` (32 páginas por IPI), `Pages`, a promessa das tabelas, `resolve_fault_with` | `kernel/src/uspace.rs`, `kernel/src/tlb.rs` (`shoot_range`) |
| `task::stop` e `kill_requested` (parar a thread), `note_retry` | `kernel/src/task.rs`; os pontos de volta ao usuário em `syscall.rs` (fim de `dispatch`), `interrupts.rs` (fim de IRQ, falta resolvida) |
| O canal de boot com VMAR raiz e VMO do initramfs | `kernel/src/process/boot.rs`, `Vm::initramfs_phys` |
| `PROCESS_BYTES_MAX`, `PROCESS_NAME_MAX`, `VMO_MAX_BYTES`, `MMAP_FLOOR`/`MMAP_ASLR_LIMIT`/`MMAP_TOP`, `USER_STACK_PAGES`, `PROCESS_KILLED_CODE` | `core/hal/src/abi.rs` |
| O carregador do lado do usuário (`spawn::create`, `Child::{stack, send_boot, start}`), `Mapping`, `Archive` | `user/rt/src/{spawn,vm}.rs`, as sete syscalls novas em `sys.rs` |
| O filho (modos: olá, laço, bloqueio, estouro de pilha, eco, saída, `process_kill` de si) e o `procs` (os casos hostis, o teto e os laços) | `user/tests/child`, `user/tests/procs`; o `init` cria o `child` |

### O processo, a thread e a morte

O `Process` é um `Arc` que cada thread e cada handle seguram; a tarefa guarda o `Arc<ThreadObj>` (que aponta o processo) em vez do ponteiro cru de antes. Um processo nasce **sem thread** (`Created`); a primeira `thread_create` o inicia, e `BadState` é o processo que morre ou morreu. O espaço mora numa `Option<Mem>` que a morte tira: depois dela `vmar_map` e `vmar_unmap` dão `BadState`. `thread_exit` da última thread encerra o processo com `Exit` de código 0.

**Pôr a thread para fora.** Quem mata marca o processo (`Dying`), grava o evento (o primeiro vale) e, para cada thread viva (uma máscara de slots de tarefa, mantida sob o lock de estado), liga o bit `kill` do slot, acorda a que espera e manda o IPI de reescalonar à CPU que a roda. A thread confere o bit em **todo ponto de volta ao usuário**, com a IRQ mascarada: o fim de `dispatch` (syscall), o fim de falta resolvida, o fim de IRQ (`preempt`) e a entrada da thread (que confere o processo, porque o bit pode ter sido ligado antes de ela existir). `object_wait_one` confere o bit a cada volta do laço, então a thread bloqueada sai da espera e o objeto só guarda um bit que a própria thread retira (isto paga a [DEBT-0111](../tech-debt/0111-espera-de-objeto-por-mascara-de-slots-de-tarefa.md) no que ela tinha de morte). Uma thread em `debug_write` só sai quando a chamada volta ([DEBT-0081](../tech-debt/0081-debug-write-de-512-bytes-com-a-irq-mascarada.md)).

**A ordem da morte** (a tabela de [Falhas de usuário](#falhas-de-usuário) vale; cada thread faz 1 a 3 e só a última faz 4 a 6):

| # | Passo |
| --- | --- |
| 1 | `begin_death` (só em quem inicia a morte: `process_exit`, falha, `process_kill`): fase `Dying`, evento, as outras threads paradas |
| 2 | `unbind_user`: a CPU volta à raiz do kernel e o bit da CPU sai da máscara do espaço; `TERMINATED` da thread |
| 3 | `thread_left`: contagem de threads; a que zera é a última |
| 4 | `teardown` (a última): tira o `Mem`, fecha todos os handles, espera o espaço inativo (não gira: todas as threads já saíram), desmapeia as páginas de VMO, devolve os frames e as tabelas, devolve as promessas |
| 5 | Fase `Dead`, `TERMINATED` do processo, a linha da morte e `LAST_EVENT` |
| 6 | Só depois de **todo local com `Drop` ter saído de escopo** a tarefa chama `task::exit` (que não volta: o que sobrasse vivo vazaria) |

Um processo sem thread que `process_kill` mata é desfeito por quem matou, na hora. Um processo que perde o último handle sem nunca ter morrido (nunca rodou) é desfeito no `Drop`. A `process_kill` em si mesmo não volta: o bit da própria tarefa liga e o fim de `dispatch` a faz sair. O código do evento de um `process_kill` é `PROCESS_KILLED_CODE` (-1024); `thread_exit` da última thread e `process_exit(0)` dão `Exit` com código 0.

**A falta de página** ganhou o lock por espaço que a dívida 0120 pedia (paga): a resolução, a troca de folha e o `vmar_unmap` rodam sob o `IrqLock` do `Mem`, que **gira atendendo os pedidos de invalidação** (`lock_serving`): quem segura o lock espera o *shootdown* das outras CPUs, e uma CPU que gira no lock responde por elas. Duas faltas seguidas que só repetem (`Retry`) na mesma página sem IRQ nem syscall no meio matam o processo, em vez de prender a CPU.

### VMO e VMAR

O VMO **é dono dos frames**; cada mapeamento segura um `Arc<Vmo>`. A falta numa região de VMO (`FaultPlan::MapObject`) pede o frame ao VMO, que o cria zerado no primeiro toque, **leitura ou escrita**, e o mapeia com a permissão da região. O VMO físico responde `base + página`. O desmapeamento tira a folha e **não** devolve o frame de VMO; a região do carregador (imagem, pilha) segue dona dos frames. O que os dois dão a quem chama:

| Chamada | Contrato preenchido |
| --- | --- |
| `vmo_create(size, flags)` | `flags` 0, `size` de 1 página a `VMO_MAX_BYTES` (256 MiB) e múltiplo de página, senão `InvalidArgs`. Cobra `256 + 4 por página` do teto do criador e promete as páginas contra os frames livres: `NoMemory` na chamada, nunca a morte na falta de página. Handle com todos os direitos de VMO |
| `vmar_map(vmar, vmo, vmo_off, len, flags, vmar_off)` | Flags inválidas ou W+X: `InvalidArgs`; `vmar` e `vmo` com `MAP` e cada permissão pedida (o direito do handle é o teto): `AccessDenied`; tipo errado: `WrongType`. `len` zero, desalinhado, `vmo_off` desalinhado, `vmar_off` sem `FIXED`: `InvalidArgs`; fora do VMO: `OutOfRange`. `FIXED` abaixo de `USER_MIN`, na faixa nula ou acima do topo: `OutOfRange`; **sobre região ocupada: `InvalidArgs`**, sem mudar nada. Sem vaga na árvore ou sem faixa livre ou sem promessa das tabelas: `NoMemory`. Sem `FIXED` escolhe a primeira faixa livre a partir do `mmap_base`, sorteado por processo em `2^28` páginas a partir de 64 GiB; sem entropia no gerador, `process_create` dá `NotSupported`. `flags` sem permissão nenhuma reserva faixa que sempre falta (a guarda do `user/rt`) |
| `vmar_unmap(vmar, addr, len)` | `MAP` no handle. Apara, parte ou remove regiões; faixa sem região é sucesso. Partir uma região sem vaga é `NoMemory` com a árvore intacta. As folhas saem e as outras CPUs do espaço são invalidadas (um IPI por 32 páginas) antes de qualquer frame voltar. Em região preguiçosa a promessa de cada página volta ao contador e a página que tinha frame deixa de contar como com frame (paga a dívida 0123) |

**A reserva estrita** (decisão 9): o VMO promete suas páginas; o `vmar_map` promete as tabelas de página do mapeamento (uma por 512 páginas e mais uma, devolvidas quando o último pedaço do mapeamento sai); `commit::reserve` confere contra os frames livres menos o que já está prometido. A [DEBT-0121](../tech-debt/0121-reserva-so-das-paginas-preguicosas.md) segue aberta só no modo por processo e no que o kernel aloca.

**O VMO não usa a página zero.** O frame do VMO é compartilhado entre espaços, e uma leitura mapeada para a página zero em um espaço ficaria velha quando outro escrevesse; por isso a falta de VMO sempre cria o frame ([DEBT-0141](../tech-debt/0141-vmo-sem-pagina-zero.md)).

### O teto em bytes por processo (dívida 0110, paga)

Cada processo tem uma conta de **`PROCESS_BYTES_MAX = 512 KiB`** de heap do kernel, cobrada a quem cria e devolvida quando o objeto some (`ByteCharge`, no `Drop`). Acima dela a chamada dá `NoMemory`:

| O que | Custo cobrado |
| --- | --- |
| Ponta de canal | 4 096 bytes (metade do par, que mede 7,2 KiB), mais a ponta na conta de 256 |
| Mensagem em fila | o payload mais 64 de cabeçalho, ao processo que escreveu, até a mensagem sair da fila |
| VMO | 256 mais 4 bytes por página (o índice de frames nasce inteiro) |
| Processo filho | 8 KiB (o objeto, a árvore de 32 regiões, os slots da tabela) |
| Thread | 4 KiB (o objeto e a área de FPU/SIMD, até uns 2,7 KiB) |

**Por que 512 KiB:** (a) abaixo de 256 pontas × 4 KiB = 1 MiB, então o teto em bytes é o que de fato morde, e o de 256 pontas fica como contagem; (b) com no máximo 24 tarefas vivas e cada processo no seu teto, o pior caso de usuário é 12 MiB de heap, menos de 5% dos 256 MiB da QEMU; (c) o `init` com um filho gasta cerca de 20 KiB, 25 vezes menos; (d) o custo de processo e de thread medido (cerca de 9,5 KB de heap com o filho esperando, e 14 a 16 frames no total) cabe nos 12 KiB cobrados. O revisor mediu 17,7 MiB de pico e 18,3 MiB vazados num processo hostil; o `procs` esgota o teto de três jeitos e vê `NoMemory` (10 pares com 311 a 322 KB de mensagens; 1 590 a 1 637 VMOs de uma página; 25 filhos), e depois de fechar tudo a conta volta e a criação segue.

O que fica fora do teto é a [DEBT-0142](../tech-debt/0142-objeto-do-kernel-sem-try-new.md): o `Arc::new` dos objetos e a lista de trabalho ainda abortam na falta de heap; o payload da mensagem e os índices usam `try_reserve`.

### O canal de boot e o `init`

A mensagem de boot do `init` leva agora `[Process, VMAR raiz, Console, Initramfs]`: o VMAR raiz com os direitos de VMAR e o initramfs como **VMO físico** (`READ`, `MAP`, `DUPLICATE`, `TRANSFER`), só para o `init` (a opção `initramfs` de `spawn_with`); os programas que o shell lança recebem só os três primeiros. O leitor casa por `kind`, então o programa que não conhece um item o ignora.

O **`init` cria um filho** (`user/init`, com `user/rt/src/spawn.rs`): mapeia o initramfs pelo VMO, acha o `child` com o `core/cpio`, valida o ELF com o `core/elf`, cria o canal de boot e o processo (`process_create`, que move a ponta do filho para a tabela dele como o handle 1), preenche um VMO por `PT_LOAD` mapeando-o em si, mapeia o mesmo VMO no filho com a permissão do segmento e o endereço do linker, monta a pilha (um VMO de 18 páginas reservado sem permissão, as 16 do meio desmapeadas e remapeadas, de modo que as páginas de cada lado ficam reservadas e sempre faltam) com o `InitialStack` do `hal`, escreve a mensagem de boot com o processo, o VMAR raiz e o console duplicado, e chama `thread_create` com o handle de boot em `rdi`/`x0`. Depois espera o `TERMINATED`. O filho escreve pelo console que recebeu e sai: as linhas `[user] child: hello from a child process` e `[user] init: the child process ran and terminated`. A ABI não diz o tamanho de um VMO, e o `init` o acha por tentativa de mapeamento ([DEBT-0140](../tech-debt/0140-abi-sem-tamanho-de-vmo.md)).

### Decisões do agente

| Decisão | O que ficou | Por quê |
| --- | --- | --- |
| Os erros que o contrato deixava abertos | Tabela de `vmar_map` acima; `process_create` com `events` diferente de 0 é `NotSupported` (o canal de eventos é da 4d), consumindo `bootstrap` e `events`; nome maior que 32 bytes é `InvalidArgs`; o nome é validado e **não** guardado ([DEBT-0134](../tech-debt/0134-ps-mostra-user-como-nome-de-tarefa-de-processo.md)); `thread_create` valida o handle antes de `entry` e `sp` | Preenche o que o contrato numérico não dizia; **a confirmar pelo dono**, é o que o mundo de fora enxerga |
| A última thread encerra o processo | `thread_exit` da última é `Exit` de código 0 | O processo sem thread não tem como voltar a rodar |
| `PROCESS_KILLED_CODE` | -1024 | Um valor que `process_exit` não usa por engano; vira o `code` do evento na 4d |
| Quem mata não espera | `process_kill` é assíncrona: devolve ao marcar e parar; o pai espera o `TERMINATED` | O Zircon também (`zx_task_kill`); a espera já existe |
| Só o bit do slot para as threads | O fim de `dispatch` e de IRQ confere um `AtomicBool` do slot, e não o processo | Uma conferência pelo processo custava 3 vezes a syscall nula no TCG ([SDD 07](07_desempenho_e_recursos.md#processos-vmo-e-vmar-fatia-4c-parte-3)) |
| O despacho segue mascarado | A IRQ não liga no despacho | A parada por IPI não precisa dela; [DEBT-0081](../tech-debt/0081-debug-write-de-512-bytes-com-a-irq-mascarada.md) reescrita |
| `Process` é um `Arc` com a tabela dentro | O `init` tem um handle de si, e o ciclo se quebra no `drain` da morte | O processo que nunca roda e perde os handles é desfeito no `Drop` |
| A pilha do filho com páginas reservadas sem permissão | Dois `vmar_map` sem flags e um `vmar_unmap` no meio | Nenhum `Attrs` de guarda na ABI; uma região sem permissão é a guarda |
| `init` fora do boot normal | Segue só no modo de teste ([DEBT-0087](../tech-debt/0087-init-so-no-boot-de-teste.md), reescrita com o custo medido) | O orçamento do boot é do dono |
| A dívida 0133 e a 0131 do M1 | Seguem abertas, com o gatilho reescrito | O shell e o `programs.rs` são do M1 |

**O que o autoteste confere** (`make smoke`, modo de teste, nas duas ISAs): os casos do VMO sem processo (cobrança, promessa, frame no primeiro toque, teto, devolução exata); o `init` com o filho; e o `procs` (`[user] procs: ok (N assertions)`, 113 hoje), com direitos errados em VMO e VMAR (sem `WRITE`, sem `MAP`, só `EXECUTE`, VMAR sem `WRITE`, o initramfs para escrita e para execução), W+X, bit reservado, tipo errado e handle forjado; `FIXED` sobre região (inteira, no meio, no fim, no começo), na faixa nula, abaixo do mínimo, no topo e na metade do kernel; tamanhos desalinhados, zerados e que dão a volta; o desmapeamento parcial (buraco no meio, as páginas em volta com o valor, remapear no buraco, o que cobre buracos e regiões) e a memória compartilhada entre dois mapeamentos do mesmo VMO; `process_create` sem `MANAGE_PROCESS`, com pai de outro tipo, `bootstrap` sem `TRANSFER`, nome longo, ponteiro de saída ruim, `events`, o mesmo handle nos dois e pai forjado, cada um consumindo os handles que se movem e sem criar nada; `thread_create` sem `MANAGE_THREAD`, com `entry` e `sp` ruins e em processo morto; `process_kill` sem `DESTROY`, duas vezes e de processo que nunca rodou; a thread que sai, o processo que acaba com a última, o `process_kill` de processo **bloqueado no kernel** (acorda pela morte e não pelo prazo, o que paga a dívida 0112), de processo com **quatro threads girando em usuário** (o kernel confere que o IPI alcançou até 3 CPUs de uma vez), de processo que se mata, e o estouro de pilha numa thread com outra girando; o teto em bytes (mensagens, VMOs, processos); 500 rodadas de criar, mapear, tocar e desmapear; e 300 processos criados e esperados (metade morta no meio) mais 2 000 idas e voltas entre dois processos. Nos pontos `A` e `C` o kernel confere que **nada sobrou**: os mesmos frames, os mesmos bytes de heap e a mesma promessa. O teste do heap exato achou, durante o desenvolvimento, o `Box` da thread que o `enter_user` (que não volta) não soltava.


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
| `EFER.SCE`, `STAR`, `LSTAR`, `FMASK` | **MSRs por CPU**: cada núcleo os grava no próprio init (um passo novo depois dos de `CR0`/`CR4`/`XCR0` da lista do [SDD 10](10_interrupcoes.md)), a CPU de boot e as secundárias |
| `STAR` | `(0x10 << 48) \| (0x08 << 32)`: com a GDT de hoje, `SYSRET` dá `SS = 0x18 \| 3` e `CS = 0x20 \| 3` (a conta `+8`/`+16` é a **verificada** acima), que são `USER_DATA` e `USER_CODE` de `segments.rs` |
| `LSTAR` | `rustos_syscall_entry`, na seção `.entry.text` ([KPTI-ready](#o-trampolim-e-o-caminho-pronto-para-kpti)) |
| `FMASK` | A máscara do Linux acima. Interrupção só liga quando a pilha da tarefa já é a do kernel |
| Pilha | A entrada guarda o `rsp` do usuário em `gs:[user_rsp]`, carrega `gs:[kernel_rsp]` e monta o **mesmo** `TrapFrame` de 22 palavras que as IRQs já usam ([SDD 15](15_tarefas_e_preempcao.md)), com `SS`, `RSP`, `R11` como `RFLAGS`, `CS`, `RCX` como `RIP`. `user_rsp` e `kernel_rsp` são campos **do bloco `PerCpu` desta CPU**, no prefixo que o `arch/` define ([SMP](#interação-com-o-smp-e-o-estado-por-cpu)) |
| `kernel_rsp` | A troca de contexto escreve o topo da pilha da tarefa que entra em dois lugares **desta CPU**: `gs:[kernel_rsp]` e o `rsp[0]` **da TSS desta CPU** (a `CpuTables` por CPU que o #13 criou; o hardware carrega `RSP0` ao subir de anel numa exceção ou IRQ). Hoje nenhuma TSS usa `rsp[0]`. Como cada CPU tem a sua TSS, a migração não cria corrida: quem escreve é sempre a CPU que troca. A troca roda mascarada, então a tarefa não migra no meio |
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
| NMI, `#DF`, `#MC`, `#DB` (IST) | Podem vir de qualquer estado: **entrada paranoica**. Decide pelo valor de `IA32_GS_BASE` (um `rdmsr`, o caminho caro): em kernel ele é o endereço do `PerCpu`, da metade alta; em usuário é o `GS` do usuário, da metade baixa (proposta; a técnica do Linux, `paranoid_entry`, **não foi lida** nesta rodada) |

**A troca para o par `swapgs`, na 4a.** O #13 deixou `IA32_GS_BASE` = `PerCpu` e
`IA32_KERNEL_GS_BASE` = 0 (SDD 10, passo 1 do init de cada núcleo). Esse estado **já é** o de
"kernel rodando, `GS` do usuário vale 0": a 4a não regrava MSR nenhum. Basta a regra "**um
`swapgs` em cada transição de anel**, e nenhuma em tarefa só de kernel":

| Momento | `IA32_GS_BASE` | `IA32_KERNEL_GS_BASE` |
| --- | --- | --- |
| Hoje, e em qualquer thread do kernel | `PerCpu` desta CPU | 0 |
| Rodando no kernel, depois da 4a (qualquer origem) | `PerCpu` (`gs:[0]` funciona, `percpu::current()` não muda) | o `GS` do usuário (0: `FSGSBASE` desligado e a fase 4 não tem TLS) |
| Rodando em anel 3 | o `GS` do usuário (0) | `PerCpu` |
| Cada `swapgs` | troca os dois | troca os dois |

O primeiro `swapgs` de uma CPU, na saída da primeira tarefa para o anel 3, produz o estado da
linha "anel 3" sem escrever MSR. **Por CPU:** cada núcleo mantém o seu par; nada é global.
`percpu::pinned` continua valendo (o `swapgs` só roda com IRQ mascarada). `CR4.FSGSBASE` fica
desligado (o usuário não escreve `FS`/`GS` base; a fase 4 não tem TLS, ver
[carregador](#executável-e-carregador)).

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
| 1 `#DB` | `Fault` (sem depuração na fase 4). **Ganha IST própria** (IST4, com guarda abaixo, **uma por CPU**: a `CpuTables` passa de 3 para 4 pilhas IST de 16 KiB), porque pode disparar na primeira instrução da entrada, antes do `swapgs` e da troca de pilha. **Vindo do anel 3, o stub da IST4 não trata nada nela:** faz `swapgs` (e o CR3 do kernel sob KPTI), **copia o quadro de hardware para a pilha da tarefa** (`gs:[kernel_rsp]`, a do `TSS.rsp[0]`) e segue como entrada normal, como a `idtentry_mce_db` do Linux ("switch stacks and treat it as a normal entry"). Matar o processo e o `schedule` rodam na pilha da tarefa, **nunca na IST** | Entrada paranoica na IST, relata e para |
| 2 NMI, 8 `#DF`, 18 `#MC` | Continuam com IST e fatais com relatório, mesmo vindos de usuário | idem |
| Qualquer outro vetor sem tratador | `Fault` | Como hoje |

**O `IRETQ` de saída pode falhar.** O Linux o diz no `entry_64.S`: "This may fault.
Non-paranoid faults on return to userspace are handled by fixup_bad_iret. These include
#SS, #GP, and #NP." Quando ocorre, a CPU já trocou `GS` e CR3 para os do usuário (o
`swapgs` e a troca de CR3 vêm **antes** do `iretq`), e o `CS` salvo é o do kernel: um stub
de exceção "do kernel" que confiasse no `CS` usaria o `GS` do usuário. O caminho
`.Lerror_bad_iret` do Linux faz `swapgs`, volta ao CR3 do kernel e chama `fixup_bad_iret`,
que monta o quadro "as if we faulted immediately after IRET" (**verificado**, cópia local).

**O que o `IRETQ` faz em modo 64 bits** (**verificado**, espelho do SDM:
[iret](https://www.felixcloutier.com/x86/iret:iretd:iretq)):

| Fato | Consequência para o desenho |
| --- | --- |
| `RFLAGS.NT = 1` **no momento do `IRETQ`** dá `#GP(0)`, antes de qualquer pop | É o `NT` **atual do kernel**, não o do quadro. O `FMASK` limpa `NT` na entrada de syscall; que a entrada de exceção também o limpe **NÃO VERIFICADO** no SDM (leitura de gate) |
| `RSP` ou `SS` não canônico no quadro: **`#SS(0)`** ("an attempt to pop a value off the stack causes a non-canonical address to be referenced"). `RIP` não canônico: `#GP(0)` | O `RSP` do usuário (a CPU o aceita em anel 3) é a fonte real de falha, e cai no vetor 12; o `RIP` cai no 13 |
| Para `CPL 3`: `IOPL`, `VIF` e `VIP` **não** são restaurados; `RF`, `AC` e `ID` são; `IF` só se `CPL <= IOPL` (o `CPL` do pseudocódigo é o de antes do retorno, o 0 do kernel: leitura nossa do espelho) | O usuário **não** ganha `IOPL` por um quadro adulterado; `NT` do quadro é restaurado sem problema |

Desenho do RustOS:

| O que | Como |
| --- | --- |
| Por construção | `CS` e `SS` são constantes do kernel; `RIP` vem de `thread_create`, validado (`USER_MIN <= entry < USER_TOP`), ou é o que a CPU salvou; `RFLAGS` vem da CPU ou, numa thread nova, é `0x202` |
| O que sobra | O `RSP` não canônico (`#SS`), e o `NT` atual do kernel, se algum caminho o deixar ligado |
| Onde se decide | **No stub dos vetores 11 `#NP`, 12 `#SS` e 13 `#GP`, em assembly, antes de tocar em `gs`:** compara o `RIP` do quadro com o endereço do `iretq` (`lea`/`cmp` relativos ao `rip`, que não usam `gs`). Igual: é o caso do *fixup*. Diferente: segue a entrada normal. O `#SS` do `RSP` não canônico cai no vetor 12, coberto |
| *Fixup* | Só depois da comparação: `swapgs`; sob KPTI, o CR3 do kernel (o stub e a IDT estão no trampolim, então rodam com o CR3 do usuário); monta o `Fault` de usuário e o processo morre |
| Teste | `thread_create` com `entry`/`sp` em `0xFFFF_8000_0000_0000`, `0x0000_8000_0000_0000` e `USER_TOP`; usuário com `rsp` não canônico e uma IRQ (esperado: `#SS(0)` no `iretq`, fixup, processo morto, kernel de pé); usuário que liga `NT = 1` e depois `AC = 1` por `popf` e faz syscall e recebe IRQ (o quadro volta com os bits, o processo **não** morre); `iretq` forçado a falhar em `#GP`, `#SS` e `#NP`, com e sem KPTI |

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

**Verificado pelo Linux** (`arch/arm64/tools/sysreg`, `pgtable-hwdef.h`, `arm_arch_timer.h`; **não** é a ARM ARM,
cujas páginas do developer.arm.com não renderizam sem JavaScript):

| Campo | Posição |
| --- | --- |
| `TCR_EL1` | `A1` bit 22; `TG0` bits 15:14 (`0b00` = 4 KiB); `EPD0` bit 7; `T0SZ` bits 5:0; `AS` bit 36 (ASID de 16 bits) |
| `ESR_ELx` | `EC` bits 31:26; `IL` bit 25; `ISS` bits 24:0 |
| Descritor de página | `AP[1]` bit 6 (`PTE_USER`, acesso de EL0), `AP[2]` bit 7 (`PTE_RDONLY`), `nG` bit 11, `PXN` bit 53, `UXN` bit 54 |
| `ID_AA64MMFR0_EL1.ASIDBits` | bits 7:4; `0b0000` = 8 bits, `0b0010` = 16 bits |
| `SCTLR_EL1.WXN` | bit 19 |
| `CNTKCTL_EL1` | bit 1 = `ARCH_TIMER_USR_VCT_ACCESS_EN` (acesso de EL0 ao contador virtual; o nome `EL0VCTEN` na ARM ARM não foi lido) |
| Acesso de usuário sob PAN | O Linux usa `ldtr`/`sttr` com `SET_PSTATE_PAN(0)`/`(1)` e `uaccess_ttbr0_enable()` (`asm/uaccess.h`): indício de que o PAN não barra `ldtr`/`sttr`, **não** a regra |

**NÃO VERIFICADO** (nem o Linux o dá): o `imm16` do SVC em `ISS[15:0]` (o desenho usa sempre
`svc #0` e não o lê); a tabela de `AP[2:1]` por nível de exceção; a regra exata do PAN com
`LDTR`/`STTR`; a versão em que surgiu o `UAO` (`ID_AA64MMFR2_EL1.UAO`, bits 7:4); se o `WXN`
torna `XN` também a página gravável de EL0; o nome `EL0VCTEN`; e um `ERET` com estado ilegal.

**Desenho (proposta):**

| Passo | O quê |
| --- | --- |
| Vetores 8 a 11 | Antes da 4a só paravam. Passam a salvar o quadro na `SP_EL1` da tarefa (a exceção vinda de EL0 usa `SP_EL1`) e a despachar por `EC` |
| `SP_EL0` | Na entrada de EL0 o quadro guarda o `SP_EL0` do usuário; a saída o restaura. **Nada do kernel usa `SP_EL0` depois disso** (o kernel roda em EL1h) |
| Pilha de falhas | **O invariante "`SP_EL0` é sempre a pilha de falhas" é falso:** entre a entrada de EL0 e o prólogo que guarda `SP_EL0`, ele é o do usuário, e o `vector_fault` do [SDD 15](15_tarefas_e_preempcao.md) (`msr spsel, #0` e empilhar) empilharia na pilha do usuário. O desenho **adota a ordem do Linux**: toda exceção síncrona ou SError de EL1h entra **na pilha da tarefa** (`SP_EL1`), com o teste de estouro de pilha do `kernel_ventry` (**verificado**: `add sp, sp, x0` e `sub x0, sp, x0`, sem registrador livre, e `tbnz` no bit `THREAD_SHIFT`); **só no estouro** vai para a pilha de falhas (o `__bad_stack`, que guarda `x0` em `tpidr_el0`/`tpidrro_el0`). O teste por bit do `sp` exige a pilha alinhada ao dobro do tamanho: pilha de 32 KiB em 64 KiB de endereço (custo só de janela, não de memória), ou comparar o `sp` com o limite guardado na área por CPU |
| Por que a ordem muda | O `vector_fault` de hoje também serve o `brk` que **volta** (`exception.rs`, [SDD 10](10_interrupcoes.md)) e passa a servir a falta recuperável do `usercopy`. Quem volta não pode empilhar numa pilha de falhas que a próxima falha reinicia do topo. Com a ordem do Linux, `brk`, `usercopy` e IRQ usam o mesmo caminho: a pilha da tarefa |
| Falta aninhada | No `__bad_stack`, se o `sp` já está **dentro** da pilha de falhas (falta enquanto relata uma falta), **continua nela**, sem reiniciar do topo |
| `TPIDR_EL1` | **Já guarda o `PerCpu` desta CPU** (#13, porta `hal::CpuLocal`), posto na entrada do núcleo, antes de qualquer falha. O topo e a base da pilha de falhas **desta CPU** entram como campos do prefixo do bloco que o `arch/` define; o caminho de estouro os lê por `mrs x, tpidr_el1`. `exception::install` e `install_secondary` **deixam de gravar o `SP_EL0`** com a pilha de falhas e gravam o topo no bloco |
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

O SMP **já existe** ([#13](15_tarefas_e_preempcao.md#estado-por-cpu)): a fase 4 roda em N núcleos desde o
primeiro PR, e o que ela guarda por CPU **entra no `PerCpu`**, não em estado global. O desenho antigo
supunha o SMP "por vir".

| | x86_64 | aarch64 |
| --- | --- | --- |
| Registrador do bloco | `IA32_GS_BASE` no kernel ([troca para o par `swapgs`](#x86_64-syscallsysret)) | `TPIDR_EL1` (**já em uso**, #13) |
| Prefixo que o `arch/` define | `PerCpu` começa por `self` (offset 0, já existe); a 4a acrescenta `user_rsp` e `kernel_rsp` logo depois. O asm de entrada só conhece esses offsets, que o `arch/x86_64` exporta; o `kernel/` embute o prefixo como o primeiro campo do `PerCpu` | `self`, mais `fault_stack_top` e `fault_stack_base`; sem `user_rsp` (o `SP_EL0` vai ao quadro) |
| TSS e pilhas | `rsp[0]` da TSS **desta CPU** a cada troca de tarefa; IST4 por CPU | A pilha de falhas por CPU, no bloco |
| Registradores de entrada | `STAR`, `LSTAR`, `FMASK`, `EFER.SCE`: por CPU, no init de cada núcleo | `VBAR_EL1`, `CNTKCTL_EL1`, `TCR_EL1`, `SCTLR_EL1`: por CPU, no init de cada núcleo |
| Raiz ativa | `CR3` de **cada** CPU; a tarefa só de kernel usa a raiz do kernel | `TTBR0_EL1` de **cada** CPU; a tarefa só de kernel usa a raiz vazia |
| Quais CPUs rodam o processo | Cada Process guarda `active_cpus` (máscara): quem o está rodando **agora** | idem |

#### PCID e ASID com várias CPUs

| Quando | Regra |
| --- | --- |
| 4a a 4d (sem PCID/ASID) | Toda troca de processo esvazia o TLB local (x86_64: `mov cr3` sem `NOFLUSH`; aarch64: um ASID reservado só e `tlbi vmalle1`). Uma CPU que o processo deixou **não guarda entrada dele**: correto e mais lento |
| 4e (com PCID/ASID) | Gerador global com **geração**; cada CPU guarda a `generation_seen`. Um valor de PCID/ASID só é **reusado** (geração nova) depois que **toda CPU** esvaziou o TLB desde que ele foi liberado: subir a geração marca `flush_pending` em todas as CPUs, e cada uma esvazia na próxima troca de processo, **antes** de carregar um valor da geração nova (x86_64: `INVPCID` tipo 2 ou 3, ou `mov cr3` sem `NOFLUSH`; aarch64: `tlbi vmalle1`) e atualiza a `generation_seen`. É o esquema do `arch/arm64/mm/context.c` do Linux, conferido pelo revisor do PR do gerador ([abaixo](#o-gerador-de-pcid-e-asid-coremmsrcasidrs)) |
| 4e, processo que migra | Com PCID/ASID o TLB da CPU antiga **guarda** o valor. Cada Process tem um `tlb_gen` que sobe a cada `unmap`; a CPU que volta a rodá-lo compara com o que ela viu e esvazia só aquele PCID/ASID (`INVPCID` tipo 1; `tlbi aside1`). A ideia é a do `tlb_gen` do Linux x86: **NÃO VERIFICADO** |

#### *Shootdown* de TLB (paga a dívida 0055 na 4c)

**Entrou na 4c** ([como entrou](#a-fatia-4c-paginação-sob-demanda-e-shootdown-como-entrou)); a tabela abaixo é o desenho e vale. Antes dele `entry_changed` só invalidava a CPU que mexeu, e isso bastava porque o kernel **só
acrescentava** mapeamentos depois do boot. A fase 4 **remove** mapeamentos de usuário.

| Ponto | Desenho |
| --- | --- |
| Quando vira pré-requisito | **4c**, com `thread_create` (threads do mesmo processo em CPUs diferentes) e `process_kill`. Em 4a e 4b (uma thread por processo, sem `unmap`, sem PCID) não há CPU com entrada velha; o `exit` libera os frames só depois de o processo sair de **todas** as CPUs |
| Dispara | `vmar_unmap` (e `vmar_protect`, se um dia existir) de um processo **com `active_cpus` além da CPU que chama**; a morte do processo com thread correndo noutra CPU |
| x86_64 | **O mecanismo de IPI por máscara já existe** (ICR por CPU, `hw_id`); os dois vetores de hoje são reescalonar (`0xFD`) e parar (`0xFE`), então **entra um vetor novo**, `Shootdown`. O iniciador grava a faixa (ou "tudo") numa caixa por CPU, manda o IPI a `active_cpus`, faz o `invlpg` local e **espera todas as confirmações antes de liberar o frame ou reusar o endereço** |
| aarch64 | `tlbi vae1is` / `aside1is`, `dsb ish`, `isb`: o domínio *inner shareable* alcança todas as CPUs **sem IPI** (a premissa da dívida 0055, não conferida no Arm ARM); a espera é o próprio `dsb ish` |
| Morte com thread em outra CPU | Marca o processo "em morte" ([ordem da morte](#falhas-de-usuário)), manda o IPI de **reescalonar** à CPU da thread e espera o `on_cpu` baixar (o campo do #13) antes de liberar a pilha de kernel e as tabelas |
| Kernel | A janela de pilhas passa a **reciclar endereço sem desmapear** (a pilha segue mapeada; só o slot volta à lista): não é remoção, não pede *shootdown*. Desmapear a pilha reabriria a dívida 0055 para o kernel |
| Teste | Duas CPUs leem uma página que uma terceira remove (o teste que a dívida 0055 pedia); depois do `unmap`, nenhuma lê o valor velho |

#### Outras dívidas do SMP que a fase toca

| Dívida | Efeito na fase 4 |
| --- | --- |
| [DEBT-0057](../tech-debt/0057-tsc-entre-nucleos-sem-verificacao.md) TSC entre núcleos sem verificação | `clock_get_monotonic` só é monotônico **entre CPUs** se o TSC for sincronizado; thread de usuário migra, então no x86_64 o relógio pode andar para trás entre duas chamadas. Aarch64 não tem o problema (`CNTVCT_EL0` é do sistema). A fase **não** paga; o contrato diz "monotônico por CPU" no x86_64 até a dívida |
| [DEBT-0054](../tech-debt/0054-heap-com-lock-global.md) Heap com lock global | O gatilho ("o primeiro programa que aloque do heap em todos os núcleos", fase 4) dispara: criar processos em N CPUs aloca objetos do kernel. Medir no `bench` da 4c; pagar se a disputa passar de 10% |
| [DEBT-0052](../tech-debt/0052-afinidade-so-no-spawn.md) Afinidade só no `spawn` | **Não dispara**: a ABI nativa não tem chamada de afinidade (o `sched_setaffinity` é do adaptador Linux, fase 5) |

## Espaço do usuário

### Layout, igual nas duas ISAs

Uma constante só (47 bits) para as duas ISAs mantém o carregador, os testes e o
`core/mm` sem `cfg` por arquitetura. `USER_MIN`, `USER_TOP` e `PAGE_SIZE` moram em
`hal::abi`, porque `arch/` e `core/mm` os usam (o `core/elf` recebe os limites do chamador, sem depender do `hal`). **Verificado** ([x86_64 mm](https://docs.kernel.org/arch/x86/x86_64/mm.html)): o
usuário do Linux vai de 0 a `0x00007fffffffefff` com 4 KiB de buraco em
`0x00007ffffffff000`. **Verificado** ([arm64 memory](https://docs.kernel.org/arch/arm64/memory.html)): `TTBR0` para o
usuário, `TTBR1` para o kernel, seleção pelo bit 55; faixa padrão de 48 bits.

| Faixa (proposta) | O quê | Sorteio |
| --- | --- | --- |
| `[0, 0x1_0000)` | **Faixa nula**: nada mapeia (o equivalente do `mmap_min_addr`; o valor do Linux **NÃO VERIFICADO**) | — |
| `0x40_0000` em diante | Imagem do `ET_EXEC`, no endereço em que o linker a pôs (`user/link/<isa>.ld`) | Nenhum na fase 4 ([decisão 5](#decisão-5--pie-de-usuário-agora-ou-depois)) |
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

A janela da pilha ficou fixada na 4b: o início da pilha cai num intervalo de 1 TiB
(`[0x6000_0000_0000, 0x6100_0000_0000)`: `2^28` páginas a partir de `0x6000_0000_0000`) e a pilha ocupa as 16 páginas seguintes, com uma guarda de cada lado. O
valor do `mmap_base` (por exemplo, um intervalo de 1 TiB acima de 64 GiB) é constante a fixar no PR da 4c.

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
Mac com HVF a CPU não anuncia `RNDR`). O boot relata a fonte e os bits efetivos (`aslr_bits`,
abaixo); o PR do kernel que liga o `random::next_u64` ainda mede a entropia de 5 000 processos
com a fonte real (teste de propriedade com o gerador próprio, como em `core/sched`).

**Como `vmar.rs` entrega isso** (implementado, testado no host): `aslr_base(floor, limit, rng)`
sorteia a base de uma região, uniforme entre `2^28` páginas (`ASLR_BITS = 28`) a partir de
`floor`, cortada em `limit`; é o `mmap_base` e o topo da pilha. Dentro da região o `ANY` usa
`Placement::Lowest` (primeira faixa livre que serve, sem entropia), como o parágrafo acima manda.
`Placement::Random` também existe: escolhe **uniformemente entre todas as posições alinhadas
que servem** (conta as posições de cada buraco, sorteia um índice e o localiza), mas o
kernel **não o usa na 4b nem na 4c**, pelo custo de tabelas de página acima. O sorteio do índice
é a multiplicação de Lemire com rejeição, no máximo **9 palavras** (1 e mais 8 rejeições):
esgotadas as rejeições, a nona palavra é usada sem conferir, para uma fonte travada não prender o
laço. A chance de chegar lá é `(limiar / 2^64)^8`, no máximo 2^-96 para até 2^52 posições. A
fonte é um `FnMut() -> u64`: o kernel passa `random::next_u64`, o teste passa um xorshift ou
palavras escolhidas à mão.

**A entropia cai sem aviso se o intervalo é curto.** Com `limit - floor` menor que `2^28` páginas,
`aslr_base` sorteia entre as páginas que cabem; `aslr_bits(floor, limit)` devolve o `log2` delas
(arredondado para baixo; 28 com o intervalo cheio; 0 se `aslr_base` daria `None`, o que inclui
intervalo sem uma página inteira) e **o boot relata esse valor** ao fixar o `mmap_base` e a janela
da pilha. O teste do host (`aslr_base_has_28_bits_of_page_entropy`) faz 20 000 sorteios com um
xorshift de semente fixa e confere: base de página, dentro do intervalo, os 28 bits baixos
todos vistos e metade dos sorteios acima do meio.

### A árvore de regiões (`core/mm/src/vmar.rs`)

A peça pura que a 4b usa direto (o carregador reserva as regiões do `init` e a pilha) e que a
4c embrulha como objeto VMAR. Ela só diz **o que está reservado e com que permissão**; quem
mapeia a página e aloca o frame é o `kernel/`. Faixa `[USER_MIN, USER_TOP)` entra por
parâmetro (`Vmar::new(storage, lo, hi)`), porque o `hal::abi` ainda não existe.

| Decisão | Valor |
| --- | --- |
| Estrutura | **Vetor ordenado por início**, em armazenamento dado de fora (`[Region; N]` ou `Box<[Region]>`: o kernel monta o `Box` no heap, sem passar pela pilha de 16 KiB). Capacidade **fixa**; cheio é `VmarError::Full`, sem alocação escondida |
| Custo ([DEBT-0075](../tech-debt/0075-vmar-com-capacidade-fixa-de-regioes.md)) | `find` e `check_fault` em **O(log N)**; `reserve` e `unmap` em O(log N) mais o deslocamento de até N regiões (`memmove`); `allocate` em **O(N)** (varre os buracos; `Random` varre duas vezes). Números em [SDD 07](07_desempenho_e_recursos.md#árvore-de-regiões-e-gerador-de-asid-fase-4-medido-no-host) |
| Memória | **32 bytes por região** (`start`, `end`, `Option<Backing>` de 8, permissões, tipo e duas guardas), fixados por `const _: () = assert!`. Armazenamento de N regiões custa `32 N`; sugestão para a 4b: N = 128 (4 KiB por processo) |
| Guarda | Uma página reservada **abaixo** e/ou **acima**, marcada na própria região (sem região extra, sem gastar vaga). Conta na sobreposição e na alocação; a falta nela é `FaultError::Guard` (estouro de pilha) e nunca resolve |
| Permissões | `Perms` (`READ`, `WRITE`, `EXECUTE`); **W+X é recusado** (`VmarError::WriteExecute`) em `reserve` e `allocate`. Região sem permissão nenhuma é válida (reserva que sempre falta) |
| O que a falta precisa | `check_fault(addr, access)` devolve a região (a falta resolve) ou `NoRegion`, `Guard`, `Permission` (o processo morre, [falhas](#falhas-de-usuário)). `Attrs.backing` guarda o **objeto e a página dentro dele** (`Backing { object: NonZeroU32, page_offset: u32 }`); sem `backing`, a região é anônima (página zero na leitura, frame novo na escrita). **Nenhum endereço físico** vive aqui |
| `unmap(start, len)` | Remove as regiões inteiras e **apara ou parte** as que o intervalo cobre em parte (o `vmar_unmap` aceita qualquer faixa). A guarda de uma ponta aparada deixa de existir; a metade alta de uma região partida soma ao `page_offset` do `backing`. Partir exige uma vaga: sem ela é `Full` **e a árvore não muda**. Faixa sem região é sucesso, sem efeito |
| Estouro | Todo `base + len` e `base - guarda` é aritmética conferida: estouro é `OutOfRange`, nunca volta ao início do espaço. Testado com uma faixa terminando em `0xFFFF_FFFF_FFFF_F000` |
| Alinhamento | `align` potência de dois, `>= PAGE` (até `2^63`); `len` e endereços múltiplos de página. A janela do pedido (`Fit.window`) limita a faixa reservada, guardas incluídas |

O `reserve` de faixa fixa é o `FIXED` do [`vmar_map`](#as-chamadas): recusa sobrepor
(`Overlap`), o equivalente do `MAP_FIXED_NOREPLACE`. Testes: modelo ingênuo (vetor ordenado,
varredura de página a página) contra 30 000 operações aleatórias com semente fixa em cinco
configurações, mais as bordas ([SDD 06](06_testes.md)).

### Página zero compartilhada e COW

| Caso | O que acontece |
| --- | --- |
| Leitura de página anônima nunca tocada | A tabela aponta para **um** frame zerado, permanente, só leitura e NX. Nunca vai ao buddy, então não precisa de contagem de referência |
| Escrita nessa página | Falta de proteção: aloca frame, zera, troca a entrada para o frame novo com escrita. É o COW a partir da página zero |
| Escrita em página anônima ausente | Aloca frame, zera e mapeia (o caso quente; o zeramento é o `DC ZVA` do aarch64 e o `rep stosq` do x86_64; a dívida 0020 (zerar pelo `memset`) foi paga no #11, e o x86_64 segue na [DEBT-0058](../tech-debt/0058-zerar-pagina-no-x86-sem-medida-em-hardware.md)) |
| COW de página já privada (`fork`, `vmo_create_child`) | **Fora da fase 4** (a ABI nativa não tem `fork`; vem com a fase 5) |

**Lido** em `mm/memory.c` do Linux (`master`, 2026-11-17), e o desenho confere com ele:

| Caso | O que o Linux faz | Linha |
| --- | --- | --- |
| Leitura de anônimo ausente | `do_anonymous_page`: "Use the zero-page for reads" (sem `FAULT_FLAG_WRITE`, e `!mm_forbids_zeropage`); a entrada é `pte_mkspecial(pfn_pte(zero_pfn(addr), …))`; trava a PTE, reconfere `vmf_pte_changed`, e grava com `set_pte_at` **sem invalidar o TLB** ("it was non-present before") | 5553, 5573–5596 |
| Escrita em anônimo ausente | "Allocate our own private page" | 5603 |
| Escrita sobre a página zero | `wp_page_copy`: `is_zero_pfn(pte_pfn(orig_pte))` escolhe o caminho da página zero (`folio_prealloc` com o aviso, **sem** copiar dados); **reconfere a PTE sob o lock** (`pte_same`); `ptep_clear_flush`, `set_pte_at`; solta o mapeamento antigo | 4077, 4098–4106, 4138, 4170–4199 |

Diferenças do RustOS: não há `userfaultfd` nem *rmap*; o frame zero é permanente e não conta
referência; **a reconferência da entrada sob o lock** (no Linux, `vmf_pte_changed` e
`pte_same`) entra no tratador, porque outra thread do processo pode ter resolvido a mesma
falta. Vale também o aviso do Linux: gravar uma entrada que estava ausente não precisa
invalidar o TLB; trocar a página zero por um frame novo **precisa** (`invlpg`/`tlbi`).
A página de um VMO físico (initramfs, framebuffer) nunca é zero nem COW.

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
| Relatório | 4e, parte 1 (**feito**) | `kpti off (not implemented, would be on)` na linha `[rustos] mitigations:` ([seção](#mitigações-de-cpu)); o KPTI de fato é a parte 2 ([DEBT-0100](../tech-debt/0100-kpti-nao-implementado.md)) |

**Quem é vulnerável.** O x86_64 liga o `X86_BUG_CPU_MELTDOWN` a menos que a CPU esteja na
lista `NO_MELTDOWN` ou tenha `RDCL_NO` (bit 0 do MSR `0x10A`); o aarch64, a menos que tenha
`ID_AA64PFR0_EL1.CSV3` ou conste da `kpti_safe_list`. O código é o `arch/<isa>/src/mitigation.rs`
([seção](#mitigações-de-cpu) traz as fontes e o estado de cada valor). Política: **sem
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
| Raiz do processo | `CR3` | `TTBR0_EL1`; `TCR_EL1.EPD0` (bit 7) volta a 0, `T0SZ` (bits 5:0) fica em 16 (48 bits, o que o Limine deixa; o mapeador só passa de `USER_TOP` se alguém pedir), `TG0` (15:14) em `0b00` (**verificado pelo Linux**; ARM ARM por ler) |
| Etiqueta | PCID nos 12 bits baixos do CR3 (`CR3_PCID_MASK = 0xFFF`, `CR3_NOFLUSH = BIT(63)`: `processor-flags.h` do Linux); o Linux liga `CR4.PCIDE` se a CPU anuncia `PCID` (`cr4_init`). **Verificado** (espelho do SDM, [mov cr3](https://www.felixcloutier.com/x86/mov-1) e [invpcid](https://www.felixcloutier.com/x86/invpcid)): com `PCIDE = 1`, o bit 63 do operando do `mov cr3` decide se o TLB é invalidado; com `PCIDE = 0`, os bits 2:0 e 11:5 do CR3 são ignorados. `INVPCID`: tipo 0 = um endereço de um PCID, 1 = um PCID inteiro, 2 = tudo (global inclusive), 3 = tudo menos o global; **`#GP`** com `PCIDE = 0` e tipo 0 ou 1 com PCID diferente de zero, e com tipo maior que 3 | ASID em `TTBR0_EL1[63:48]` quando `TCR_EL1.A1 = 0`; largura em `ID_AA64MMFR0_EL1.ASIDBits[7:4]` (`0b0000` = 8, `0b0010` = 16) e `TCR_EL1.AS` (bit 36) escolhe 16 bits (**verificado pelo Linux**) |
| Troca de processo | Escreve CR3 com o PCID do processo; sem esvaziar o TLB | Escreve `TTBR0_EL1` com o ASID; sem `tlbi` |
| Alocação | Gerador global de ASID/PCID, com contador de geração: esgotou, esvazia tudo e começa outra geração. Módulo puro em `core/mm/src/asid.rs`, testado no host ([abaixo](#o-gerador-de-pcid-e-asid-coremmsrcasidrs)); o estado por CPU (`active`, `reserved`, `flush_pending`) mora dentro dele, indexado pela CPU | idem |
| Com KPTI | Dois PCIDs por processo (raiz de kernel e raiz de usuário): o custo é metade do espaço de PCID | O ASID já distingue; o `USER_ASID_FLAG` do Linux marca a variante de usuário |
| Fallback | Sem PCID, todo CR3 esvazia o TLB: correto e mais lento | Sem ASID útil, `tlbi` por troca |
| `PageFlags` | `user` liga U/S (bit 2) | `user` liga o acesso de EL0 e `nG`; o kernel segue com UXN e PXN |

Folhas de usuário: x86_64 `U/S = 1`, NX salvo se a permissão for execução; com SMEP o
kernel não executa, com SMAP não acessa sem `AC`. aarch64: código de usuário com
`PXN = 1, UXN = 0`; dado de usuário com `UXN = 1`. **O W^X do usuário já é do mapeador**
(`MapError::WriteExecute`): `vmar_map` e as permissões de `vmar` recusam W+X.

#### O gerador de PCID e ASID (`core/mm/src/asid.rs`)

`AsidAllocator<WORDS, CPUS>` é o gerador global da regra de geração
([PCID e ASID com várias CPUs](#pcid-e-asid-com-várias-cpus)). Implementado e testado no host;
a ligação (o lock, o `CR3`, o `INVPCID`, o `tlbi`) é da 4e. A regra foi escrita a partir da
descrição do SDD; o revisor independente a conferiu contra o
[`arch/arm64/mm/context.c`](https://raw.githubusercontent.com/torvalds/linux/master/arch/arm64/mm/context.c)
do Linux, e a conferência pegou um caso: o `flush_context` do Linux diz que, no esgotamento,
"we must preserve its reserved ASID, as this is the only trace we have of the process it is
still running", então a CPU ociosa mantém o `reserved` por **vários** esgotamentos
(`double_rollover_idle_cpu_keeps_index`). A outra evidência é a simulação de TLB abaixo.

**Fora do `AsidAllocator`:** a migração entre CPUs (o `tlb_gen` por processo, que esvazia só o
PCID/ASID de quem volta a uma CPU que o guardou) **não é do gerador**; fica para a 4e.

| Ponto | Decisão |
| --- | --- |
| Valor | `Asid = geração << bits | índice`, nunca zero (a geração começa em 1). A largura `bits` é parâmetro de execução (PCID 12; ASID 8 ou 16), limitada pela capacidade `WORDS * 64` do bitmap; índices abaixo de `first` nunca saem (PCID 0, o ASID da raiz vazia). Com KPTI o chamador usa `bits - 1` e deriva os dois valores do processo |
| Estado por CPU | No lugar da `generation_seen`, um `flush_pending` por CPU **dentro** do gerador (a mesma regra: sobe a geração, toda CPU esvazia antes de carregar valor novo), mais `active` (o que a CPU roda) e `reserved` (o que ela rodava no último esgotamento) |
| Chamada | `switch(cpu, current) -> Switch { asid, flush }`: a CPU vai rodar o espaço que tem `current` (`None` se nunca teve). Se `current` é da geração em curso, volta igual; senão o gerador renova. `flush` manda esvaziar o TLB desta CPU **antes** de carregar `asid` |
| Renovar | Se o valor velho estava **reservado** por alguma CPU (rodava no esgotamento), todas as reservas dele passam ao valor novo e o **índice se mantém**; senão tenta o mesmo índice, se livre; senão toma o próximo. Quem renova **sempre** recebe um valor novo (a geração difere) |
| Esgotamento | Sem índice livre: sobe a geração, esvazia o bitmap, **reserva o que cada CPU roda agora** e marca `flush_pending` em todas. Exige-se mais índices utilizáveis que CPUs (`TooFewIds`), então o esgotamento sempre acha lugar |
| **Sem liberação individual** | Não há `release`. O espaço que morre apenas larga o valor; o índice só volta no esgotamento seguinte. Reusar um índice na mesma geração deixaria entrada velha no TLB de outra CPU, sem esvaziamento que a cobrisse. É o que o Linux faz com o ASID |
| Geração | `u64`, soma 1 por esgotamento. Se `geração << bits` estoura: `GenerationOverflow`, **sem mudar o estado** (inalcançável na prática: 2^48 esgotamentos com 16 bits) |
| Memória | `AsidAllocator<64, 8>` (PCID de 12 bits, 8 CPUs, `hal::MAX_CPUS`) ocupa **672 bytes**; `<1024, 8>` (ASID de 16 bits) ocupa **8 352 bytes** (cerca de 8,2 KiB). `const fn new`: vive num `static`, sem pilha |
| Fast path | O gerador é `&mut self`: o kernel o põe atrás de um lock e **toda troca o toma**. O desvio sem lock (como o do Linux: `active` atômico por CPU) não existe; é a [DEBT-0076](../tech-debt/0076-asid-sem-caminho-rapido-sem-lock.md) |

**Evidência.** `fuzz` roda 20 000 trocas aleatórias em 4 CPUs e 40 espaços (nascem e morrem),
com 4 a 6 bits de índice (muitas gerações), em cinco sementes (a CPU 0 quase não troca, para deixar um espaço rodando por vários
esgotamentos), e confere: o valor devolvido é da
geração em curso; **dois espaços vivos da mesma geração nunca dividem um índice**; quem estava
em geração velha recebe valor diferente; a geração sobe de 1 em 1 e, ao subir, o bitmap tem no
máximo `CPUS + 1` índices (esvaziou) e a CPU da virada esvazia. A prova central é uma
**simulação de TLB**: cada CPU guarda, por índice, o espaço que o carregou, esvazia quando
`flush` manda, e o teste exige que **nenhuma CPU carregue um índice que guarda entrada de outro
espaço**. Mais uma invariante: o índice carregado numa CPU que ainda roda o espaço é igual ao
do valor atual dele. O mesmo roteiro com o `flush` ignorado **falha** (o teste prova que a
simulação pega o erro), e a invariante também falha se o `reserved` de uma CPU ociosa se perde
(mutação testada à mão).

## Mitigações de CPU

O [SDD 08](08_roadmap.md) e o [SDD 09](09_seguranca.md) prometem estas mitigações para a
fase 4, porque mexem na entrada e na saída de syscall. Todas entram na **4e**, com o custo
medido ([SDD 07](07_desempenho_e_recursos.md)); a regra de cada uma é "liga onde a CPU
precisa, relata no boot". As fontes de cada uma estão no SDD 09; a pesquisa desta fase
lê as páginas do Linux `spectre.rst` e `mds.rst` e o `nospec.h` (**verificado** abaixo, com os
ajustes que a leitura obrigou); o que depende de custo ou de condição que elas não dão segue
**NÃO VERIFICADO**.

### O que existe hoje (4e, parte 1)

**Já é assim.** Cada ISA decodifica o que a CPU anuncia num módulo puro, testado no host, e decide
as vulnerabilidades por uma função pura; o `core/hal` guarda o tipo e a linha do boot.

| Peça | Onde |
| --- | --- |
| `Vulns` (meltdown, spectre v2, ssb, mds, bhb), `KptiPolicy`, `Mitigations` e o texto da linha | `core/hal/src/mitigation.rs` |
| A máscara do clamp (`asm!`, a sequência do Linux) | `arch/x86_64/src/nospec.rs` e `arch/aarch64/src/nospec.rs`; o `core/hal/src/nospec.rs` tem só a versão pura, de referência e de teste |
| x86_64: `MitigationFeatures::from_leaves`, `ArchCapabilities` e `vulnerabilities()` | `arch/x86_64/src/mitigation.rs`; a leitura do CPUID e do MSR em `sysreg.rs` |
| aarch64: `MitigationFeatures::from_regs` e `vulnerabilities()` | `arch/aarch64/src/mitigation.rs`; a leitura dos registradores em `protection.rs` |
| A linha e o `kpti=` | `kernel/src/mitigation.rs`; `BootParams::kpti` em `core/hal/src/cmdline.rs` |

**A linha `[rustos] mitigations:`** sai **nos dois modos**, logo depois de `protections:` (mesmo
critério dela: o `make smoke` e o `make smoke-fast` a exigem, e ela avisa o que a CPU precisa e o
kernel ainda não tem; custa uma linha, 0,3 a 0,4 ms na serial da QEMU, mais o CPUID de cada núcleo (cerca de 1 ms
no x86_64 em TCG, nada no aarch64; [SDD 07](07_desempenho_e_recursos.md)). O formato é
`v1 on, kpti <estado>, v2 <estado>, ssb <estado>, mds <estado>, bhb <estado>`, sem endereço, com
`<estado>` em:

| Estado | Quer dizer |
| --- | --- |
| `on` | O kernel liga a mitigação. Hoje só o `v1` (o clamp de índice). `Switch::On` e `Switch::Untested` são os estados que as dívidas 0100 a 0104 preveem para quando forem pagas |
| `on (hardware)` | A CPU é afetada e o próprio hardware a mitiga, sem nada do kernel: hoje só o BHB com `ECBHB` no aarch64 (o ramo `supports_ecbhb` do Linux, estado `BHB_HW`) |
| `off (not needed)` | A CPU não tem a vulnerabilidade, pela regra da tabela abaixo |
| `off (not implemented)` | A CPU precisa e o kernel não tem a mitigação ([DEBT-0101](../tech-debt/0101-spectre-v2-nao-implementado.md) a [DEBT-0104](../tech-debt/0104-bhb-nao-implementado.md)) |
| `off (untested)` | Ligada no código, mas nenhum ambiente a exercita (nenhuma hoje; o tipo `Switch::Untested` já existe) |

O KPTI diz mais: `off (not implemented, would be on)` (a CPU tem Meltdown e o kernel o ligaria),
`off (not implemented, would be on by kpti=on)` (forçado numa CPU sem Meltdown), `off (kpti=off)`
(a CPU tem Meltdown e a linha de comando mandou não ligar) e `off (not needed)`. A chave é
`kpti=on|off|auto` na linha de comando do Limine (valor desconhecido vale `auto`); **nesta fatia só
muda o texto**: quem liga é o PR do KPTI ([DEBT-0100](../tech-debt/0100-kpti-nao-implementado.md)).
**A linha é a união dos núcleos:** o de boot e cada secundário somam o `Vulns` do seu CPUID ou
registradores (o pior de cada vulnerabilidade, como o Linux, que avalia cada CPU) antes de ficar
online, e a linha sai depois de todos subirem; o núcleo que não subiu não entra
([DEBT-0107](../tech-debt/0107-mitigacoes-sem-os-nucleos-que-nao-subiram.md)). O modo de teste imprime também
`[rustos] cpu features: …`, **do núcleo de boot**, com o que o CPUID ou os registradores
anunciaram (PCID, INVPCID, IBPB, STIBP, SSBD, `MD_CLEAR`, `ARCH_CAPABILITIES`; `CSV2`, `CSV3`,
`SSBS`, `ECBHB`, `CLRBHB`).

**A regra de cada vulnerabilidade** é a do Linux, na tag `v6.12`. Marca por valor: **Verificado** é o
código do Linux lido na tag; **Verificado (secundária)** é espelho ou modelo de terceiro.

| ISA | Vulnerabilidade | Regra | Fonte |
| --- | --- | --- | --- |
| x86_64 | Meltdown | `!NO_MELTDOWN(vendor, family)` e `!RDCL_NO`; `NO_MELTDOWN` é toda a AMD e a Hygon | **Verificado:** `arch/x86/kernel/cpu/common.c`, `cpu_set_bug_bits` ("`if (x86_arch_cap_msr & ARCH_CAP_RDCL_NO) return;`") e `cpu_vuln_whitelist` |
| x86_64 | Spectre v2 | `!NO_SPECTRE_V2`; só o Zhaoxin e o Centaur de família 7 | **Verificado:** `common.c` (`VULNWL(ZHAOXIN, 7, …, NO_SPECTRE_V2 \| …)`) |
| x86_64 | SSB (v4) | `!NO_SSB` e `!SSB_NO` (bit 4) e `!AMD_SSB_NO` (`0x80000008` EBX bit 26); `NO_SSB` é Silvermont, Airmont, KNL, KNM, Yonah e a AMD de família `0xF` a `0x12` | **Verificado:** `common.c` |
| x86_64 | MDS | `!NO_MDS` e `!MDS_NO` (bit 5); `NO_MDS` é Goldmont, a AMD e a Hygon | **Verificado:** `common.c` |
| x86_64 | BHI | `!BHI_NO` (bit 20) e `!NO_BHI` e (eIBRS ou hipervisor); eIBRS é `IBRS_ALL` (bit 1) ou `AutoIBRS` (`0x80000021` EAX bit 8) | **Verificado:** `common.c` ("When virtualized, eIBRS could be hidden, assume vulnerable") |
| x86_64 | Sem especulação | Família 4, 5 da Intel e do Centaur, Bonnell e Saltwell: nada a mitigar | **Verificado:** `common.c`, `intel-family.h` (`0x1C`, `0x26`, `0x27`, `0x35`, `0x36`) |
| x86_64 | Sem a folha 1 | `Vulns::ALL` (o Linux a trata como família 4; o RustOS não confia nisso) | Decisão do agente |
| aarch64 | Meltdown | `CSV3 == 0` e fora da `kpti_safe_list` | **Verificado:** `arch/arm64/kernel/cpufeature.c`, `unmap_kernel_at_el0` |
| aarch64 | Spectre v2 | `CSV2 == 0` e fora da `spectre_v2_safe_list` | **Verificado:** `arch/arm64/kernel/proton-pack.c` |
| aarch64 | SSB (v4) | Fora da `spectre_v4_safe_list`; `SSBS` só diz que o controle existe | **Verificado:** `proton-pack.c` (`spectre_v4_get_cpu_hw_mitigation_state`) |
| aarch64 | BHB | Afetada se `CSV2 != 3` e (`CLRBHB`, ou nas listas de laço ou de firmware do Linux, ou em nenhuma lista); com `ECBHB` ela é afetada **e mitigada por hardware** (`on (hardware)`). **Diverge do Linux** na última parte: ele só marca a CPU das listas | **Verificado:** `proton-pack.c` (`is_spectre_bhb_affected` não consulta o `ECBHB`; `spectre_bhb_enable_mitigation` tem o ramo `supports_ecbhb`), `cpufeature.h` (`supports_csv2p3`, `supports_clearbhb`) |
| aarch64 | MDS | Não existe | **Verificado:** o Linux só define `X86_BUG_MDS` |

**Os registradores e bits**, todos com a posição **Verificada** em `arch/arm64/tools/sysreg` (v6.12) e
**Verificada (secundária)** no espelho da Arm em
[`df.lth.se/~getz/ARM/SysReg/`](https://df.lth.se/~getz/ARM/SysReg/AArch64-id_aa64pfr0_el1.html)
(que traz o texto "FEAT_CSV2_3 is implemented" e "FEAT_ECBHB implements the functionality identified
by the value 0b0001"): `ID_AA64PFR0_EL1.CSV3` [63:60] e `CSV2` [59:56] (3 é `CSV2_3`),
`ID_AA64PFR1_EL1.SSBS` [7:4], `ID_AA64MMFR1_EL1.ECBHB` [63:60], `ID_AA64ISAR2_EL1.CLRBHB` [31:28]
(`S3_0_C0_C6_2`, codificado no `asm!` porque o assembler base não o conhece) e `MIDR_EL1`
(`Implementer` [31:24], `Architecture` [19:16], `PartNum` [15:4]; o Linux só casa lista com
`Architecture == 0xF`). No x86_64, **Verificado** em `cpufeatures.h` (v6.12): `CPUID.1:ECX` bit 17
(`PCID`) e 31 (hipervisor); `CPUID.7.0:EBX` bit 10 (`INVPCID`); `CPUID.7.0:EDX` bit 10 (`MD_CLEAR`),
26 (`IBRS`+`IBPB`), 27 (`STIBP`), 29 (`ARCH_CAPABILITIES`), 31 (`SSBD`); `CPUID.80000008H:EBX`
bit 12 (`AMD_IBPB`), 14 (`AMD_IBRS`), 15 (`AMD_STIBP`), 24 (`AMD_SSBD`), 25 (`VIRT_SSBD`), 26
(`AMD_SSB_NO`); e em `msr-index.h`: `IA32_ARCH_CAPABILITIES` `0x10A` com `RDCL_NO` 0, `IBRS_ALL` 1,
`RSBA` 2, `SSB_NO` 4, `MDS_NO` 5, `TAA_NO` 8, `SBDR_SSDP_NO`/`FBSDP_NO`/`PSDP_NO` 13 a 15,
`FB_CLEAR` 17, `RRSBA` 19, `BHI_NO` 20, `PBRSB_NO` 24, `GDS_NO` 26, `RFDS_NO` 27;
`IA32_SPEC_CTRL` `0x48` (`IBRS` 0, `STIBP` 1, `SSBD` 2, `BHI_DIS_S` 10) e `IA32_PRED_CMD` `0x49`
(`IBPB` 0). A **leitura do `MSR 0x10A` só ocorre com o `CPUID.7.0:EDX[29]`**, como no Linux
(`x86_read_arch_cap_msr`). O que o SDM e o ARM ARM dizem desses bits **NÃO foi lido** (os
campos vêm do Linux e do espelho).

**Os testes** de `vulnerabilities()` usam valores de CPUs reais: no x86_64, os dumps de CPUID e
MSR do repositório [InstLatx64](https://github.com/InstLatx64/InstLatx64) (commit `17be9a9`: Haswell,
Skylake, Cascade Lake, Ice Lake, Sapphire Rapids, Alder Lake, Goldmont, Silvermont, Zen 2, Zen 4,
Bulldozer, Hygon); no aarch64, os registradores dos modelos da QEMU 10.0.0 (`target/arm/tcg/cpu64.c`),
que copiam os de núcleos reais (Cortex-A35, A55, A72, A76, Neoverse N1, V1, N2, A710 e A64FX). Cada
bit tem também um teste sintético, marcado como tal. As listas são **parte** das do Linux
([DEBT-0106](../tech-debt/0106-listas-de-cpu-do-linux-so-em-parte.md)).

**O que a QEMU mostra** (`make smoke`, `release`; o modelo de CPU é o `max` e as outras são
`RUSTOS_CPU`): na TCG o CPUID **não anuncia** `SPEC_CTRL` nem `ARCH_CAPABILITIES` nem nos modelos
`Skylake-Server` e `Cascadelake-Server`, então estes saem como CPU sem informação (vulnerável).

```text
x86_64, TCG, -cpu max (AMD família 6):
[rustos] mitigations: v1 on, kpti off (not needed), v2 off (not implemented), ssb off (not implemented), mds off (not needed), bhb off (not needed)
x86_64, TCG, -cpu Cascadelake-Server:
[rustos] mitigations: v1 on, kpti off (not implemented, would be on), v2 off (not implemented), ssb off (not implemented), mds off (not implemented), bhb off (not implemented)
aarch64, HVF (Apple M5):
[rustos] mitigations: v1 on, kpti off (not needed), v2 off (not needed), ssb off (not implemented), mds off (not needed), bhb off (not implemented)
aarch64, TCG, -cpu a64fx:
[rustos] mitigations: v1 on, kpti off (not implemented, would be on), v2 off (not implemented), ssb off (not implemented), mds off (not needed), bhb off (not implemented)
```

### A lista de mitigações da fase

**Estado** hoje: o clamp está **feito**; as demais são o desenho da parte 2 ou da primeira máquina em metal, cada uma com a dívida que a deixa `off (not implemented)` na linha.

| Mitigação | Contra | Onde entra | Custo |
| --- | --- | --- | --- |
| Clamp de índice (Spectre v1) | Leitura especulativa fora da tabela | **Feito** para o número da syscall (índice da tabela de despacho de `kernel/src/syscall.rs`); falta o índice do handle na tabela do processo (a tabela entra na 4c). **Sempre ligado**, em toda CPU | **Verificado** (`include/linux/nospec.h`): `array_index_nospec` calcula uma máscara **sem desvio**, `~(long)(index \| (size - 1UL - index)) >> (BITS_PER_LONG - 1)`, e faz `index & máscara` (índice fora do limite vira 0); uso: `if (index < size) { index = array_index_nospec(index, size); val = array[index]; }`. **Medido:** custo no x86_64 e no aarch64 em [SDD 07](07_desempenho_e_recursos.md); a sequência por ISA é a do Linux (`arch/<isa>/src/nospec.rs`; no aarch64 com `CSDB`) ([DEBT-0105](../tech-debt/0105-csdb-do-clamp-custa-10-ns-sem-o-requisito-lido.md)) |
| IBPB | Spectre v2 entre processos | **Condicional, não em toda troca**: o Linux só o emite para tarefa com especulação de desvio indireto restringida (estado `IBPB: conditional`: "Use IBPB on SECCOMP or indirect branch restricted tasks"). No RustOS: na troca **de e para** processo marcado como restrito, se a CPU anuncia a instrução | A medir (o SDD 09 não tem número na fonte) |
| eIBRS ou retpoline | Spectre v2 no kernel | **eIBRS é o preferido** onde a CPU tem ("should use the feature … instead of retpoline. Enhanced IBRS is more efficient than retpoline": `spectre.rst`); retpoline só onde não há. O kernel é compilado soft-float: o retpoline exige conferir o alvo de build | A medir |
| STIBP | Spectre v2 entre *hyperthreads* | Só com SMT, e só se a política pedir | A medir |
| Limpeza de buffers na saída para usuário | MDS | `mds.rst`: "The mitigation for MDS clears the affected CPU buffers on return to user space and when entering a guest" (o documento **não nomeia o `VERW`** nessa frase). O `VERW` aparece em `arch/x86/include/asm/nospec-branch.h` ("Macro to execute VERW insns that mitigate transient data sampling") e o Linux tem `CLEAR_CPU_BUFFERS` antes do `sysretq` (**verificado** no `entry_64.S`). Liga se a CPU é vulnerável e não anuncia `MDS_NO` | A medir |
| SSBD | Spectre v4 | Por processo, sob pedido (política); padrão desligado | A medir |
| KPTI | Meltdown | [Trampolim](#o-trampolim-e-o-caminho-pronto-para-kpti) | ~100 ciclos por troca de CR3 |

**O clamp, como está no código.** A máscara de cada ISA mora em `arch/<isa>/src/nospec.rs`, com
`asm!` e a sequência do Linux, **Verificada** na tag `v6.12` (`arch/x86/include/asm/barrier.h`:
`cmp; sbb`; `arch/arm64/include/asm/barrier.h`: `cmp; sbc` e `csdb`, com o comentário de que a
máscara é `~0` quando `0 <= idx < sz`). O `kernel/src/platform/<isa>.rs` faz `index & máscara`. O
`core/hal/src/nospec.rs` guarda só a versão pura (aritmética com `black_box`), que os testes do host
conferem e que serve de referência; o kernel não a chama. O despacho de `kernel/src/syscall.rs` virou
uma tabela indexada pelo número (de 0 a 0x42, vazios para os números que a ABI não define; uma
asserção de compilação confere que ela e `Syscall::from_raw` concordam), e o número passa pela
conferência de limite e depois pelo clamp, como no `nospec.h`. **Conferido no assembly**
(`objdump -d` do `syscall::dispatch`, `release`, `rustc` 1.100.0): x86_64 `cmp %rdx,%rcx; sbb
%rsi,%rsi; and %rcx,%rsi`; aarch64 `cmp; ngc` (o `sbc xzr, xzr` desmontado), `csdb`, `and`. Como a
máscara é `asm!`, o compilador não pode trocá-la por desvio. O requisito da `CSDB` e o motivo dela
**NÃO foram lidos** no Arm ARM ([DEBT-0105](../tech-debt/0105-csdb-do-clamp-custa-10-ns-sem-o-requisito-lido.md)).

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
(valores numéricos de direitos, assinaturas completas, a profundidade da fila do canal)
**não é premissa**.

| Fato verificado | Fonte |
| --- | --- |
| Direitos (DUPLICATE, TRANSFER, READ, WRITE, MAP, EXECUTE, WAIT, INSPECT, GET/SET_PROPERTY, DESTROY, MANAGE_PROCESS, MANAGE_THREAD) e a possibilidade de reduzi-los por `zx_handle_replace` e `zx_handle_duplicate` (a página diz que os direitos "can be reduced"; **inferência do RustOS:** "só diminuem", que ela não enuncia assim) | [rights](https://fuchsia.dev/fuchsia-src/concepts/kernel/rights) |
| `zx_handle_replace(handle, rights, out)`: direitos novos devem ser subconjunto (senão `ZX_ERR_INVALID_ARGS`); **"handle is always invalidated"** (a página não separa sucesso de erro; **inferência do RustOS:** vale também no erro); direitos exigidos: **"None"**; erros `BAD_HANDLE`, `INVALID_ARGS` e `NO_MEMORY` | [handle_replace](https://fuchsia.dev/fuchsia-src/reference/syscalls/handle_replace) |
| `zx_channel_write`: **"All handles are discarded and no longer available to the caller, on success or failure"**; o canal exige `WRITE` e cada handle do array exige `TRANSFER`; handle repetido no array dá `BAD_HANDLE`; a **própria ponta** do canal no array dá `NOT_SUPPORTED`; limites: 65 536 bytes e 64 handles por mensagem | [channel_write](https://fuchsia.dev/fuchsia-src/reference/syscalls/channel_write) |
| `zx_process_create(job, name, …)` devolve o handle do processo e o do VMAR raiz; a thread só roda após `zx_process_start`; exige `ZX_RIGHT_MANAGE_PROCESS` no job | [process_create](https://fuchsia.dev/fuchsia-src/reference/syscalls/process_create) |
| `zx_channel_create`: dois handles; mensagens de um lado são lidas no outro; transferência de handle com `ZX_RIGHT_TRANSFER`. **Ponta fechada:** "messages in flight to that endpoint are discarded and any Handles they contained are closed"; leitura de canal vazio com a outra ponta fechada dá `PEER_CLOSED` (a página de `channel_read` **não** diz que a mensagem fica na fila em `BUFFER_TOO_SMALL`: isso é desenho do RustOS, não fato lido) | [channel_create](https://fuchsia.dev/fuchsia-src/reference/syscalls/channel_create), [conceitos](https://fuchsia.dev/fuchsia-src/concepts/kernel/concepts), [channel_read](https://fuchsia.dev/fuchsia-src/reference/syscalls/channel_read) |
| `zx_process_start(process, thread, entry, stack, arg1, arg2)`: `arg1` é um handle **transferido** ao processo novo (o valor dele entra em `arg1` da thread); **"If zx_process_start() returns an error, arg1 is closed rather than transferred"**; "all other registers are zero upon start" | [process_start](https://fuchsia.dev/fuchsia-src/reference/syscalls/process_start) |
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

Proposta, sem equivalente verificado no Zircon. O handle tem **64 bits** (decisão 2):

| Item | Desenho |
| --- | --- |
| Valor | `u64` = `geração << 16 \| (índice + 1)`: geração de 48 bits, índice de 16. **`0` nunca é válido** (o índice mais um é no mínimo 1). A geração começa em 0, então o primeiro handle de um processo vale `1` |
| Geração | 48 bits, sobe a cada reuso do slot. **Aposentadoria:** o slot cuja geração chegaria a `0xFFFF_FFFF_FFFF` é aposentado e nunca mais é usado, então a geração nunca dá a volta e handle velho jamais aponta para outro objeto. Teste de propriedade: nenhum valor de handle reaparece |
| **Orçamento da geração** | Cada slot aguenta 2^48 − 1 usos; a tabela, 65 534 slots, soma cerca de 2^64. A 1 M de handles por segundo, **um único slot reciclado sem parar** se aposenta em cerca de 8,9 anos (2^48 ÷ 10^6 s), e a tabela inteira leva cerca de 585 mil anos. O esgotamento por geração deixou de ser risco prático; a [decisão 2](#decisão-2--abi-nativa-lista-números-nomes-e-contrato) trocou os 32 bits (cerca de 2^32 handles na vida do processo, 71 minutos àquela taxa) pelos 64 |
| Teto | 4 096 handles **em tabela** por processo (proposta); passar disso é `NoMemory`. O índice vai até 65 534 (16 bits). **O handle dentro de uma mensagem em fila não conta nesses 4 096** (por isso há também o [limite de canais](#destruição-de-um-canal)) |
| Slot | **32 bytes**, não 16 nem 24. `Object` é um enum fechado de `Arc<Process>`, `Arc<Thread>`, `Arc<Vmo>`, `Arc<Vmar>`, `Arc<ChannelEnd>` e `Arc<Console>`: tag mais ponteiro dão **16 bytes** (medido com `rustc` na revisão); somam-se `rights: u32` e `generation: u64` (a geração de 48 bits), 28 bytes, e o alinhamento fecha em 32. Com a geração de 16 bits o slot era de 24; encolher a tag e o ponteiro (campos separados, com `unsafe` para remontar o `Arc` a cada acesso) não adianta, porque a geração sozinha já passa de 16. **Sem `dyn`** em todo caso: `Arc<dyn Objeto>` tem ponteiro gordo (16) e o slot passa de 32. Pior caso do teto: 4 096 × 32 = 128 KiB por processo, e a tabela cresce por demanda |
| Tabela | `Vec` de slots mais lista de livres, cresce por demanda; protegida por **um `IrqLock` por processo** |
| Referências | O `Arc` segura a **memória** do objeto (handle, mapeamento e fila de mensagem); o **ciclo de vida lógico** vem da contagem de handles, abaixo. Filho para pai usa `Weak`: sem ciclo de `Arc` |
| Contagem de handles | Cada objeto guarda `handle_count`: handles em tabela **e em trânsito** (dentro de uma mensagem). Chega a zero: o objeto recebe `on_zero_handles()` |
| `on_zero_handles` | Canal: fecha a ponta ([abaixo](#destruição-de-um-canal)). VMO: nada (os mapeamentos o seguram; os frames voltam quando o último `Arc` solta). Process e Thread: nada (vivem até sair) |

#### Destruição de um canal

A destruição é por **contagem de handles**, não pelo último `Arc`.

| Passo | O quê |
| --- | --- |
| 1 | Quando a contagem de uma ponta chega a zero, ela **morre** (o Zircon confirma: ao fechar a ponta, "messages in flight to that endpoint are discarded and any Handles they contained are closed"): esvazia a fila **dela** (as mensagens destinadas a ela) e fecha os handles que estão nessas mensagens, em laço com lista de trabalho (sem recursão: fechar um handle pode matar outra ponta, que esvazia outra fila) |
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
primeiro; **erro nele também consome os movidos** (alinhado ao Zircon: `zx_process_start` fecha o `arg1` em **qualquer** erro, e `channel_write` descarta os handles "on success or failure"); só **`ShouldWait`** e a falha ao **ler** o array do usuário (o kernel ainda não sabe quais handles são) não consomem; (2) os handles **movidos** são tratados como um
array: validar **todos**, depois remover **todos** de uma vez, sob o lock da tabela;
(3) o mesmo valor duas vezes (`bootstrap == events`, ou repetido no array) é `BadHandle`;
(4) a partir da remoção, em erro, os removidos são **fechados**; (5) o handle que o kernel
guarda (o `events`) entra em `handle_count` **na criação** do processo, senão a ponta
morreria ali.

| Operação | Regra |
| --- | --- |
| `handle_close` | Remove o slot. Não pede direito. Se foi o último handle do objeto, `on_zero_handles` roda **depois** de soltar o lock da tabela |
| `handle_duplicate(h, rights)` | `h` exige `DUPLICATE`; `rights ⊆ h.rights`; insere o handle novo e devolve o valor |
| `handle_replace(h, rights)` | Remove `h` **sempre** que ele existe, em sucesso e em erro (lido no Zircon: "handle is always invalidated", sem separar sucesso de erro; a extensão ao erro é inferência). Se `rights ⊄ h.rights`: `InvalidArgs`, com o original já invalidado. `h` que não existe: `BadHandle` (nada a invalidar). **Nenhum direito é exigido** (lido: "Rights: None"); em particular, não pede `DUPLICATE` |
| `process_create(parent, …, bootstrap, events)` | `parent` com `MANAGE_PROCESS` (erro: `bootstrap` e `events` são consumidos, como o `arg1` do Zircon). `bootstrap` (`TRANSFER`) e `events` (`WRITE` e `TRANSFER`) seguem a regra geral. O `bootstrap` vai para a tabela do filho; o `events` fica **no Process**, contado em `handle_count`, e o kernel o fecha depois de escrever o evento final (então o pai vê `PEER_CLOSED` depois do evento) |
| `channel_write(ch, bytes, handles)` | Abaixo |
| `channel_read(ch, …)` | Abaixo |

**`channel_write`**, na ordem (o array de handles já foi **copiado** para o kernel, ver
[cópia usuário↔kernel](#cópia-usuáriokernel)). A ordem de locks é **tabela, depois canal**,
nunca o contrário:

1. Sob o lock da tabela, valida `ch`: existe, é canal, tem `WRITE`. Se não: erro, e os handles do array que existem **são removidos e fechados** (como no Zircon; antes este desenho poupava quem errava o handle do canal, e a divergência não tem fonte que a justifique).
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
| Atomicidade do `channel_write` | Handle repetido, a própria ponta, sem `TRANSFER`, `PeerClosed` e o excesso de bytes ou de handles consomem o array por inteiro; `ch` inválido consome o array (os handles que existem saem e fecham) |
| **`ShouldWait` não consome** | Encher a fila de uma ponta; escrever com um handle: `ShouldWait`, e o handle **segue na tabela**; esperar `WRITABLE`; reescrever: sucesso, e o handle sai da tabela |
| `process_create` | `bootstrap == events` é `BadHandle` e consome os dois; o `events` entra em `handle_count` (a ponta do pai não vê `PEER_CLOSED` na criação); erro em `parent` consome `bootstrap` e `events`, como o `arg1` do `zx_process_start` |
| `channel_read` | Cópia que falha (ponteiro ruim no meio do buffer) deixa a mensagem na fila e a tabela intacta; duas threads lendo o mesmo canal não perdem nem duplicam mensagem |
| Ciclo, na escrita | Escrever o handle da ponta oposta pela própria ponta mata a oposta já na escrita |
| Geração | Nenhum valor de handle reaparece, inclusive depois da aposentadoria do slot |

### Direitos iniciais ([decisão 3](#decisão-3--o-conjunto-inicial-de-bits-de-direito))

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
| `vmo_create`, `channel_create` | Criam objeto novo do chamador. **`vmo_create` gasta memória do sistema**: sem handle de recurso, um processo esgota a RAM. Na fase 4 a defesa é a contabilidade da [decisão 9](#decisão-9--overcommit-na-abi-nativa); o limite **por processo** é da fase 9, e até lá um processo pode reservar a RAM inteira e negar `NoMemory` aos outros |
| `random_get` | Entrega bytes do gerador do kernel, como o `getrandom` do Linux. Segredo do kernel não sai: o gerador troca a chave a cada pedido ([SDD 09](09_seguranca.md)) |

Processo nasce com **um** handle: o canal de boot.

## ABI nativa mínima

Tudo desta seção é **Proposta**; a numeração, os nomes e o contrato foram aprovados na
[decisão 2](#decisão-2--abi-nativa-lista-números-nomes-e-contrato).

### Convenção de chamada

| | x86_64 | aarch64 | Fonte |
| --- | --- | --- | --- |
| Instrução | `syscall` | `svc #0` | **Verificado** ([syscall(2)](https://man7.org/linux/man-pages/man2/syscall.2.html)) |
| Número | `rax` | `x8` | idem |
| Argumentos 1 a 6 | `rdi`, `rsi`, `rdx`, `r10`, `r8`, `r9` | `x0` a `x5` | idem |
| Retorno | `rax` = status, `rdx` = valor | `x0` = status, `x1` = valor | Os dois registradores de retorno existem nas duas ISAs ([syscall(2)](https://man7.org/linux/man-pages/man2/syscall.2.html): `rax`/`rdx`, `x0`/`x1`) |
| Preservados | Todos, exceto `rax`, `rdx` e **`rcx`/`r11`** (o hardware os sobrescreve) | Todos, exceto `x0`, `x1` | `rcx`/`r11`: **verificado** no `entry_64.S` |
| Status | `0` = `Ok`; negativo = erro | idem | **Proposta** (o Linux devolve `-errno`; o espaço de números não é copiado) |

`vmo_create` devolve o handle em `rdx`/`x1`. `channel_create` e `process_create` devolvem
**dois** handles, e dois `u64` não cabem em `rdx`/`x1`: o par vai por **ponteiro de saída**
(`out`, dois `u64` consecutivos). A falha ao gravar `out` desfaz a criação ([L4](#lacunas-a-fechar-no-pr-da-4c)).

### Entrada do programa

O contrato de como um programa **começa**, igual para o `init` e para qualquer filho.

| Item | x86_64 | aarch64 |
| --- | --- | --- |
| Modo | CPL 3; `CS` e `SS` de usuário; `RFLAGS = 0x202` | EL0t; `DAIF` limpo |
| `arg0`, `arg1` | `rdi`, `rsi` (os argumentos de `thread_create`) | `x0`, `x1` |
| Handle de boot | `process_create` **move** o `bootstrap` para a tabela do filho como o **primeiro handle, de valor `1`**. Quem chama `thread_create` passa `arg0` (o `user/rt` passa `1`); para o `init`, o kernel põe `arg0 = 1` (**feito na parte 1 da 4c**; na 4b era `0`) | idem |
| Demais registradores | Zero, **`rdx` inclusive** (o psABI o lê como ponteiro de `atexit`) | Zero (a glibc lê o `x0` como `rtld_fini`: [conflito](#a-fatia-4b-como-entrou)) |
| Pilha | `rsp = sp` de `thread_create`, **alinhado a 16 na entrada**: o psABI diz "guaranteed to be 16-byte aligned at process entry" (**verificado**, [x86-64 psABI](https://github.com/hjl-tools/x86-psABI), `low-level-sys-info.tex`) | `SP_EL0 = sp`, `sp mod 16 = 0` ("The stack must be quad-word aligned", **verificado**, [AAPCS64](https://github.com/ARM-software/abi-aa)) |
| Conteúdo da pilha | O `init`: o kernel monta `argc = 0`, `argv`, `envp` e o [auxv](#auxv-mínimo). Filho: quem o criou (o `user/rt` do pai) monta o que quiser; o `user/rt` do filho lê o auxv se houver | idem |
| SIMD | Estado inicial do [SDD 15](15_tarefas_e_preempcao.md): `FCW` `0x037F`, `MXCSR` `0x1F80` | V0–V31, `FPSR`, `FPCR` zerados |
| TLS | Nenhum: `FS`/`GS` base em zero | `TPIDR_EL0` zero |

### Códigos de erro

| Código | Valor | Quando |
| --- | --- | --- |
| `Ok` | 0 | Sucesso |
| `InvalidArgs` | -1 | Argumento fora do domínio (inclui mais de 1 024 bytes ou 8 handles numa mensagem de canal), bit reservado, tamanho zero, desalinhado, `entry`/`sp` fora de `[USER_MIN, USER_TOP]`, direitos que não são subconjunto |
| `BadHandle` | -2 | Valor que não é handle vivo (inclui geração velha) ou handle repetido num array |
| `WrongType` | -3 | Handle de outro tipo de objeto |
| `AccessDenied` | -4 | Falta direito no handle |
| `NoMemory` | -5 | Sem frame, sem slot, passou o teto de handles, ou a contabilidade negou ([decisão 9](#decisão-9--overcommit-na-abi-nativa)) |
| `BadState` | -6 | **Só** processo já morto ou em morte (`thread_create`, `process_kill`). Não existe "thread já iniciada": `thread_create` cria **e inicia** |
| `NotSupported` | -7 | Número de syscall desconhecido; flag que a fase não implementa; a própria ponta do canal no array de handles; recurso do kernel indisponível (`random_get` com o gerador sem semente, `clock_get_monotonic` sem relógio) |
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
| `0x03` | `debug_write` | `console`, `ptr`, `len` | bytes escritos | `console`: `WRITE` |
| `0x04` | `clock_get_monotonic` | — | ns (`u64`), ver [contrato](#contrato-numérico) | Nenhum |
| `0x05` | `random_get` | `ptr`, `len` (até 256) | — | Nenhum |
| `0x10` | `handle_close` | `h` | — | Nenhum |
| `0x11` | `handle_duplicate` | `h`, `rights` | handle novo | `h`: `DUPLICATE`; `rights ⊆ h.rights` |
| `0x12` | `handle_replace` | `h`, `rights` | handle novo | **Nenhum** (lido no Zircon); `rights ⊆ h.rights` |
| `0x13` | `object_wait_one` | `h`, `signals`, `deadline_ns` | sinais observados | `h`: `WAIT` |
| `0x20` | `vmo_create` | `size`, `flags` (0) | handle do VMO | Nenhum (ver acima) |
| `0x21` | `vmar_map` | `vmar`, `vmo`, `vmo_off`, `len`, `flags`, `vmar_off` | endereço mapeado | `vmar`: `MAP`; `vmo`: `MAP` e cada permissão pedida (`READ`/`WRITE`/`EXECUTE`), todas também no teto do `vmar` |
| `0x22` | `vmar_unmap` | `vmar`, `addr`, `len` | — | `vmar`: `MAP` |
| `0x30` | `process_create` | `parent`, `name_ptr`, `name_len`, `bootstrap`, `events`, `out` | — (processo e VMAR raiz, dois `u64`, em `out`) | `parent`: `MANAGE_PROCESS`; `bootstrap`: `TRANSFER` (move); `events`: `WRITE` e `TRANSFER` (move; 0 = nenhum) |
| `0x31` | `thread_create` | `process`, `entry`, `sp`, `arg0`, `arg1` | handle da thread | `process`: `MANAGE_THREAD`. Cria **e inicia**; a primeira thread inicia o processo |
| `0x32` | `process_kill` | `process` | — | `process`: `DESTROY` |
| `0x40` | `channel_create` | `flags` (0), `out` | — (as duas pontas, dois `u64`, em `out`) | Nenhum |
| `0x41` | `channel_write` | `ch`, `bytes`, `nbytes`, `handles` (array de `u64`), `nhandles` | — | `ch`: `WRITE`; cada handle: `TRANSFER`. [Consome o array](#operações-sobre-handle) |
| `0x42` | `channel_read` | `ch`, `bytes`, `bytes_cap`, `handles` (array de `u64`), `handles_cap`, `actual*` | — (tamanhos em `actual`: dois `u64`, `{ bytes, handles }`) | `ch`: `READ`. Só retira a mensagem **depois** da cópia ([ordem](#operações-sobre-handle)) |

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

**Handle:** `u64`, `0` inválido, geração de 48 bits e índice de 16 com aposentadoria do slot, teto de 4 096 em tabela por processo e **orçamento de 2^48 − 1 usos por slot** ([tabela de handles](#tabela-de-handles)).

**Limites do canal.** Comparados com o Zircon (**verificado**: 65 536 bytes e 64 handles por mensagem; a profundidade da fila **NÃO VERIFICADO**):

| | RustOS (proposta) | Zircon |
| --- | --- | --- |
| Bytes por mensagem | 1 024 | 65 536 |
| Handles por mensagem | 8 | 64 |
| Mensagens na fila de cada ponta | 16 | não lido |
| Pior caso por ponta | 16 KiB de bytes mais 128 handles | — |

**Limites e formas que a parte 1 da 4c fixou** (preenchem o que a tabela acima e as chamadas deixavam aberto; os números de `Status`, de syscall e o layout da mensagem de boot não mudam):

| Item | Contrato |
| --- | --- |
| Array de handles de usuário | Até **64** entradas (`USER_HANDLE_ARRAY_MAX`); acima disso `InvalidArgs` **sem consumir** (o kernel não leu quais são). De 9 a 64, o teto do canal vale: `InvalidArgs` **consumindo** o array |
| Pontas de canal por processo | **256** (`MAX_CHANNEL_ENDS`), cobradas ao criador enquanto a ponta existir; a 257ª é `NoMemory` em `channel_create` |
| `actual*` do `channel_read` | **Obrigatório** (nulo ou fora de `UserRange` é `OutOfRange`) e aponta para `ReadActual`, `#[repr(C)]` de 16 bytes: `bytes: u64` no offset 0 e `handles: u64` no 8. É gravado antes de a mensagem sair da fila e também em `BufferTooSmall`, com o que a mensagem pede. **A confirmar pelo dono** (o contrato original dizia só "tamanhos em `actual`") |
| `signals` de `object_wait_one` | Bit além dos quatro definidos é `InvalidArgs`, como `signals == 0` |
| `deadline_ns` de `object_wait_one` | Acima de `agora + 2^61` ns (`WAIT_HORIZON_NS`, cerca de 73 anos) vale **sem prazo**, como `u64::MAX`: o timer do hardware nunca recebe valor que dê a volta no contador |
| `handle_replace` com direitos inválidos | Bit reservado ou valor acima de 32 bits é `InvalidArgs` **e o handle velho morre** (existindo), como no erro de subconjunto; sem handle, `BadHandle` |
| Ordem de validação | Em `channel_read` e `debug_write`, o handle primeiro e depois o resto |

São menores de propósito (a meta de memória): a mensagem de boot cabe, e dado maior vai por
VMO. O custo é que um protocolo que precise de mensagem grande tem de usar VMO.

**`clock_get_monotonic`:** nanossegundos desde o boot do kernel (o relógio do [SDD 10](10_interrupcoes.md): TSC calibrado no x86_64, `CNTVCT_EL0` no aarch64), monotônico, `u64`, sem suspensão a descontar. **No x86_64 só é monotônico por CPU** enquanto a [DEBT-0057](../tech-debt/0057-tsc-entre-nucleos-sem-verificacao.md) estiver aberta (o TSC de cada núcleo não é conferido). Resolução: a do contador. **`deadline_ns`** é absoluto nesse relógio; `deadline <= agora` consulta sem bloquear (devolve os sinais se já valem, senão `TimedOut`); `u64::MAX` espera sem fim. Não há acordar falso: a chamada reavalia os sinais antes de voltar.

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
partir daí o que existir é contrato ([decisão 2](#decisão-2--abi-nativa-lista-números-nomes-e-contrato)).

### Variante enxuta (16 chamadas)

Sem `thread_create`, `thread_exit` e `process_kill`, e com `process_start` no lugar (uma
thread por processo): 16 chamadas, ainda acima de "11 a 15". Custo: sem multithread
nativo até a fase 5, e o pai só mata o filho pelo canal ou esperando.

## Executável e carregador

### Formato: ELF64 estático ([decisão 1](#decisão-1--formato-do-executável-nativo))

| Item | Valor | Fonte |
| --- | --- | --- |
| `e_machine` | `62` (x86_64), `183` (aarch64) | **Verificado**: `include/uapi/linux/elf-em.h` |
| Classe e ordem | 64 bits, little-endian: `EI_CLASS` (índice 4) = `ELFCLASS64` (2); `EI_DATA` (índice 5) = `ELFDATA2LSB` (1); `EI_VERSION` (6) = `EV_CURRENT` (1); `EI_OSABI` (7) = `ELFOSABI_NONE` (0); `EI_NIDENT` = 16 | **Verificado**: [gABI](https://www.sco.com/developers/gabi/latest/ch4.eheader.html). **Testado:** o `rust-lld` grava `EI_OSABI = 3` num programa com `#[used]` ([DEBT-0074](../tech-debt/0074-elf-recusa-osabi-gnu-do-used.md)) |
| Tipo | `ET_EXEC`; `ET_DYN` é recusado na fase 4 (decisão 5) | — |
| Seções | O kernel **nunca** lê section headers | **Proposta** |

### Validação, regra por regra

O molde é `fs/binfmt_elf.c` do Linux (**verificado**, [fonte](https://github.com/torvalds/linux/blob/master/fs/binfmt_elf.c)).
A validação mora em `core/elf` (`forbid(unsafe_code)`, `&[u8]` para uma `Image`; sem dependência: o chamador passa `Limits { machine, user_min, user_top, page_size, max_loads_bytes }`, e o `kernel/` os tira de `hal::abi`),
testada no host com ELF sintético, dois ELF reais e os casos inválidos (guidelines 09). As decisões de implementação estão no [SDD 03](03_mapa_de_codigo.md#formatos-de-entrada-corecpio-e-coreelf).

| Regra | De onde |
| --- | --- |
| `e_ident` começa com `ELFMAG` | Linux (verificado) |
| `e_type` é `ET_EXEC`; `ET_DYN` recusado na fase 4 ([decisão 5](#decisão-5--pie-de-usuário-agora-ou-depois)) | Linux (verificado) |
| Máquina e classe batem com a ISA do kernel (`elf_check_arch`) | Linux (verificado; `EI_CLASS`/`EI_DATA` estão dentro dela, não visíveis no trecho lido) |
| `e_phentsize == size_of::<Phdr>()` | Linux (verificado) |
| `e_phoff + e_phnum * e_phentsize` sem estouro e **dentro do arquivo** (a tabela de program headers inteira) | **Proposta** (própria) |
| Tamanho total dos program headers entre 1 e 65 536 bytes | Linux (verificado) |
| `p_filesz <= p_memsz` | Linux (verificado) |
| `p_align` é 0, 1 (sem alinhamento, gABI) ou potência de dois | Linux (verificado) e gABI |
| `PT_INTERP` **recusado** (o Linux o aceita, limitado e terminado em NUL; a fase 4 não tem interpretador) | Linux (verificado) e decisão do RustOS |
| `p_offset + p_filesz` sem estouro e dentro do arquivo | Proposta |
| `PT_LOAD` com `p_memsz == 0` **recusado** (não mapeia nada; é erro do linker) | Proposta |
| `p_vaddr + p_memsz` sem estouro, **e os limites conferidos depois de arredondar à página**: início arredondado para baixo e fim arredondado para cima, ambos dentro de `[USER_MIN, USER_TOP]`, sem estouro no arredondamento | Proposta |
| `p_vaddr ≡ p_offset` **módulo o tamanho da página** (não módulo `p_align`) | **Verificado**: [gABI, ch. 5](https://www.sco.com/developers/gabi/latest/ch5.pheader.html): "loadable process segments must have congruent values for p_vaddr and p_offset, modulo the page size". Sobre `p_align` a gABI diz só que 0 e 1 significam "sem alinhamento" |
| **Nenhum `PT_LOAD` com W e X** (W^X por segmento) | Proposta; é o que o mapeador já exige |
| `PT_LOAD` ordenados por `p_vaddr`, sem sobreposição (em granularidade de página) | Proposta |
| `e_entry` dentro do conteúdo de arquivo (`p_filesz`) de um `PT_LOAD` executável; a cauda zerada não vale | Proposta |
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

**Pilha inicial.** x86-64, **verificado** no psABI: `rsp` aponta `argc`; acima vêm os ponteiros de
`argv`, um zero, os de `envp`, um zero, e as entradas do auxv (dois `u64` cada) terminadas por
uma entrada nula. O desenho monta isso para o `init`. **aarch64:** o AAPCS64 só exige
`sp mod 16 = 0`; o layout inicial do processo não é dele. O Linux o faz em `fs/binfmt_elf.c`
(`STACK_ROUND(sp, items)` alinha a 16) e o RustOS usa **a mesma ordem** (`argc`, `argv`,
`envp`, auxv), mas isso é **o Linux, não uma especificação independente: NÃO VERIFICADO**.
A entrada do programa recebe também o handle do canal de boot em `rdi`/`x0` (**Proposta**, no
espírito do `zx_process_start`, que **verificado** transfere um handle como `arg1`).

### Como o `init` e os outros programas chegam à memória

| Quem | Como |
| --- | --- |
| `init` | O **kernel** o carrega: `core/elf` valida; para cada `PT_LOAD` aloca frames, copia os bytes do arquivo, deixa o resto zerado (`.bss`, inclusive a cauda da última página de arquivo) e os mapeia, com a permissão do segmento, **direto no espaço do processo** pela árvore de regiões de `core/mm` (4b: **sem objeto VMO nem VMAR e sem handle**; o VMO e o VMAR raiz como objetos só existem a partir da 4c, e então o `init` passa a receber o handle do VMAR raiz). Cópia, não mapa em lugar: os dados do `newc` só alinham a 4 bytes |
| Outros programas | Quem os cria. O `init` lê o ELF do initramfs (que recebeu como VMO), cria o processo, e preenche e mapeia os VMOs com as chamadas da ABI; o `user/rt` usa o mesmo `core/elf` |

Só o `init` está no caminho de confiança do kernel (initramfs do bootloader; boot
verificado é da fase 12). O carregador do usuário **não** é fronteira de segurança: o que
o kernel garante (W^X, faixas, direitos) vale na `vmar_map`, qualquer que seja o parser.

### O que muda no linker e no alvo

[SDD 05](05_build_e_execucao.md): os dois alvos são `x86_64-unknown-none` e
`aarch64-unknown-none-softfloat`, Tier 2, sem `build-std`. Os programas de usuário usam os
mesmos alvos, com flags diferentes. **Verificado** nesta rodada (fonte do `rustc`, `compiler/rustc_target/src/spec/targets/`):

| Alvo | O que a especificação já define |
| --- | --- |
| `x86_64-unknown-none` | `code_model: Some(CodeModel::Kernel)`; `static_position_independent_executables: true`; o `relocation_model` não é definido (padrão `Pic`) |
| `aarch64-unknown-none-softfloat` | `relocation_model: RelocModel::Static`; sem `code_model` |
| `CodeModel` | `tiny`, `small`, `kernel`, `large`: **`small` é valor reconhecido** |

**Testado** (`rustc 1.100.0`, 2026-11-17, um `#![no_std] #![no_main]` mínimo com um `static mut` e um
`static` de leitura, ligado com `rust-lld` num script que põe `.text` em `0x40_0000` e uma página
por permissão, `-z max-page-size=0x1000`):

| Alvo e flags | Resultado |
| --- | --- |
| x86_64, `-C code-model=small -C relocation-model=static` | Compila e liga: `ET_EXEC`, `EM_X86_64`, entrada `0x40_0000`, `PT_LOAD` em `0x40_0000` (`R E`) e `0x40_1000` (`RW`), acessos por `rip` |
| x86_64, `-C code-model=kernel` (o padrão do alvo), mesmas demais flags | **Também liga e gera o mesmo código** para esse programa mínimo, porque o endereço baixo (< 2 GiB) cabe no modelo. O teste **não distingue** os dois modelos: em programa maior os endereços ainda cabem, então o `small` é a escolha correta, não uma necessidade provada |
| x86_64, `-C relocation-model=pic` ou `pie`, `code-model=small` | Liga, mas o executável cresce (13 344 contra 8 848 bytes): tem GOT; é o caminho da decisão 5 |
| aarch64, `-C code-model=small -C relocation-model=static` | Compila e liga; `adrp`/`ldr`, entrada `0x40_0000`. Sem a flag, o padrão do alvo já é `static` |

**Na 4b** (medido nas duas ISAs com o `init` e o `big`): o `small` com a lista completa liga e roda; o LTO fat falha no x86_64 com o `core` pré-compilado em `kernel` e o `xtask` desliga o LTO ali ([DEBT-0084](../tech-debt/0084-usuario-do-x86-sem-lto-por-causa-do-code-model.md)); a lista de flags é `-C relocation-model=static`, `-C code-model=small` e `-C lto=off` (x86_64), `-C strip=debuginfo`, `-C link-arg=-T<script>`, `-zmax-page-size=0x1000` e `-znorelro`.

Precedência de `rustflags` (**verificado** na doc do Cargo, `config.md`, **e testado** com
`cargo build -v`): são **quatro fontes mutuamente exclusivas**, e vale a primeira que existir:
`CARGO_ENCODED_RUSTFLAGS`, `RUSTFLAGS`, `target.<triple>.rustflags` (e `target.<cfg>.rustflags`),
`build.rustflags`. **Não se somam.** No teste, com o `config.toml` pondo `relocation-model=pie` e
`-C link-arg=-nostartfiles` no alvo, `RUSTFLAGS` e `CARGO_ENCODED_RUSTFLAGS` fizeram o
`rustc` receber **só** as flags do ambiente (zero ocorrências de `nostartfiles`); sem eles, o
`config.toml` valeu inteiro.

**O que muda no plano do xtask.** O `.cargo/config.toml` de hoje põe `relocation-model=pie` nos
dois alvos, para o kernel. O build de usuário passa **a lista inteira** de flags por
`CARGO_ENCODED_RUSTFLAGS` (separador `\x1f`): `-C code-model=small` (x86_64), `-C relocation-model=static`,
o linker script de usuário, `-z max-page-size=0x1000`, e qualquer flag que o `config.toml` passe a ter e
que o usuário também queira, **repetida à mão**, porque a do `config.toml` desaparece. O build do kernel
não muda. A consequência prática: o xtask não pode confiar em `.cargo/config.toml` para o
usuário, e um teste do xtask confere a linha de comando do `rustc` (o `-v`) contra a lista esperada.
Os programas ainda precisam de um `--target-dir` separado (`target/user/<isa>`), senão o cache
do kernel é invalidado a cada troca de flags.

## Init

O `init` é o primeiro processo e o único que o kernel lança sozinho.

### Origem ([decisão 4](#decisão-4--de-onde-vem-o-init))

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

O leitor (`core/cpio`) é um iterador sem alocação: confere o magic (`070702` é recusado),
cada dígito hexadecimal, `c_namesize` (de 1 a 4096) e `c_filesize` contra o fim do buffer
(sem estouro), o NUL do nome e a presença do `TRAILER!!!`, e usa o alinhamento da gramática
acima. Devolve só arquivo regular (`c_mode & S_IFMT == S_IFREG`) com `c_nlink == 1`; **pula**
diretório sem dado e rejeita diretório com dado, link simbólico, dispositivo, FIFO, socket e
hard link. `find` percorre o arquivo inteiro e, com nome repetido, vale a última ocorrência.
O `init` é o arquivo de nome `init`, sem `./`. Decisões e testes: [SDD 03](03_mapa_de_codigo.md#formatos-de-entrada-corecpio-e-coreelf).

**Limine.** O protocolo **saiu do repositório do bootloader**: em `v12.10.0` do
[Limine](https://github.com/Limine-Bootloader/Limine) só há `CONFIG.md`; o `PROTOCOL.md` está em
[`limine-protocol`](https://github.com/Limine-Bootloader/limine-protocol), ramo `trunk`, **sem
etiqueta**. Mas o `bootstrap` da `v12.10.0` fixa o `limine-protocol` no commit
`877c2d4c50e170575f8b8e7fbf7b8fa89b1dc157`, que é o topo do `trunk` (conferido em 2026-11-18): o
`PROTOCOL.md` e o `include/limine.h` nesse commit são os do binário em uso.
O que mudou desde a v8.x, lido nos dois:

| Item | v8.x | `limine-protocol` (`trunk`) |
| --- | --- | --- |
| Pedido | `LIMINE_MODULE_REQUEST` | **`LIMINE_MODULE_REQUEST_ID`**, id `{ COMMON_MAGIC, 0x3e7e279702be32af, 0xca1c4f3bd1280cee }` (o mesmo) |
| Módulo interno | `{ path, cmdline, flags }` | `{ path, string, flags }` (`cmdline` virou `string`) |
| `limine_file` | `path`, `cmdline`, …, `tftp_ip` | `path`, **`string`**, …, **`tftp_ipv4[4]`** |
| `INTERNAL_MODULE_COMPRESSED` | "Deprecated … may panic" | honrado se a revisão da resposta for 2 ou maior |
| Resposta | `module_count` e `modules` (ponteiros para `limine_file`) | idem; o módulo carregado é marcado `Executable/Modules`, não `Usable` |

**Conferido contra o 12.10.0 na peça do módulo (4b)**, no `common/protos/limine.c` da `v12.10.0` (o que roda de
verdade) e no `include/limine.h` e `PROTOCOL.md` do commit acima:

| O quê | Verificado |
| --- | --- |
| Pedido | Id `{ COMMON_MAGIC, 0x3e7e279702be32af, 0xca1c4f3bd1280cee }`. Revisão 0 é `id[4]`, `revision`, `response` (48 bytes); a revisão 1 acrescenta `internal_module_count` e `internal_modules`. O `limine.c` valida só o prefixo até `internal_module_count` (`get_request_rev0`), então o kernel pede **revisão 0, com 48 bytes**, sem módulo interno (só os do `limine.conf`) |
| Resposta | `limine.c` responde com **revisão 2**; `module_count` e `modules` (ponteiro para um vetor de ponteiros para `limine_file`). Sem módulo carregado **não há resposta** (o `limine.c` desfaz a alocação e não grava `response`). O kernel lê só os campos da revisão 0 |
| `limine_file` | 112 bytes: `revision`, `address` (8), `size` (16), `path` (24), `string` (32), `media_type`, `unused`, `tftp_ipv4[4]`, `tftp_port`, `partition_index`, `mbr_disk_id` e três UUIDs de 16 bytes a partir do 64. `address` é **endereço virtual no HHDM** (`reported_addr` soma `direct_map_offset`), logo `físico = address − HHDM`; o PROTOCOL.md garante 4 KiB de alinhamento e que cada módulo tem para si todos os pedaços de 4 KiB que cobre. `string` é a do `module_string` (vazia se não há) |
| Memória | O módulo é alocado como `MEMMAP_KERNEL_AND_MODULES`. O PROTOCOL.md diz que essas entradas são "ilustrativas", sem garantia de alinhamento nem de não sobreposição entre si; as `Usable` e `BootloaderReclaimable` nunca sobrepõem outra entrada, e é nisso que o adaptador se apoia para recusar um módulo que toque RAM do alocador |
| Testes | `boot/limine/src/protocol.rs` confere id e `size_of`/`offset_of` contra esses valores; `lib.rs` recusa endereço nulo, abaixo do HHDM, desalinhado, tamanho zero ou acima de 1 GiB, transbordo, fim acima do mapa de memória, e sobreposição a RAM, ao framebuffer e a outro módulo; `string` sem NUL em 257 bytes ou que não é UTF-8 vira vazia; `core/mm` testa `overlaps` (imagem) e `map_module` |

**`limine.conf`** (**verificado**, `CONFIG.md` da `v12.10.0`): `module_path` ("the path to a module
(such as initramfs)", repetível) e `module_cmdline` (**alias** de `module_string`, o nome canônico).

Como o módulo cai em `KernelAndModules`, ele **fica fora do mapa direto** ([SDD 13](13_memoria_virtual.md)). O desenho:

| Passo | O quê |
| --- | --- |
| `boot/limine` | **Feito (4b, peça do módulo).** Novo pedido de módulo; `hal::BootInfo` ganha `modules()` (porta nova no `hal`, decisão do agente) e devolve `BootModule { start, len, string }`; o `BootRecord` copia os três |
| `kernel/src/vm.rs` | **Feito.** `Vm::map_initramfs` mapeia a faixa do módulo de `string` `initramfs` só leitura e NX, numa janela própria (a terceira, de 1 GiB, depois das de dispositivos e de pilhas; [SDD 13](13_memoria_virtual.md#o-módulo-initramfs)); o W^X confere. Sem módulo, o boot segue |
| Conteúdo do initramfs | **Feito (4b).** O `xtask image` monta `target/initramfs/<isa>.image.cpio` com o `init` e, na imagem com `selftest`, o `big`; o autoteste lê o cpio e confere que os dois são ELF (`initramfs: 2 files, N bytes`, [SDD 04](04_boot.md)). O `hello.txt` provisório saiu |
| Initramfs como VMO | **A partir da 4c** (na 4b o kernel só mapeia a faixa e lê o `init` dela): o arquivo vira um **VMO físico** (frames que já existem, sem zerar) com `READ`, `MAP`, `DUPLICATE`, `TRANSFER`, e vai ao `init` pelo canal de boot |
| Devolver ao buddy | Quando o último handle do VMO fechar, devolver os frames do módulo. Que o protocolo permita reusar a memória `Executable/Modules` que não é da imagem **NÃO VERIFICADO** (o PROTOCOL.md não diz); enquanto isso o initramfs **fica residente**: [DEBT-0080](../tech-debt/0080-initramfs-residente-sem-devolver-os-frames.md) |

### O canal de boot

Mensagem única, escrita pelo kernel antes de iniciar o `init`, com um par
`(tipo, handle)` por item: o Process do próprio `init` (todos os direitos de processo),
o VMAR raiz, o console de depuração (`WRITE`, `DUPLICATE`, `TRANSFER`) e o VMO do
initramfs. O formato é um `#[repr(C)]` em `hal::abi`, igual no kernel e no `user/rt`. O
mesmo mecanismo serve a qualquer pai: ele escreve a mensagem e passa a ponta do canal em
`process_create(.., bootstrap, ..)`.

## Falhas de usuário

Antes da 4a toda exceção não recuperável parava a CPU. O desenho muda o destino de quem veio de usuário.

| Distinção | Como |
| --- | --- |
| Origem | x86_64: `CS` salvo com `RPL = 3` (o mesmo teste do `swapgs`). aarch64: grupo do vetor (EL inferior, 8 a 11). `hal::Fault` ganha `from_user: bool` e o endereço da falta |
| Regra | **Falha de usuário mata o processo; o kernel segue.** Falha do kernel continua parando, com relatório |

| Evento vindo de usuário | O que acontece |
| --- | --- |
| Falta de página dentro de um VMAR, com permissão que serve | Resolve (página zero ou frame novo) e volta; o tratador **nunca bloqueia** (sem swap nem arquivo mapeado), roda mascarado como o resto (**feito na 4c**) |
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

### Canal de eventos ([decisão 6](#decisão-6--falha-de-usuário-canal-de-eventos-ou-sinal-síncrono))

O pai passa uma ponta de canal em `process_create(.., events)`. Quando o processo termina,
o kernel escreve **uma** mensagem de `ProcessEvent` nela:

| Campo | Para `Exit` | Para `Fault` |
| --- | --- | --- |
| `kind` | `Exit` | `Fault` |
| `code` | o argumento de `process_exit` | — |
| `fault_kind` | — | falta de página, instrução ilegal, acesso desalinhado, aritmética, breakpoint, outro |
| `pc`, `sp`, `addr` | — | do usuário |
| `isa_code` | — | o `code` cru da ISA (vetor e erro; `ESR_EL1`) |

Regras: **só endereço de usuário** na mensagem (o endereço do kernel não sai: o `Addr` e o `Scrubber` do [SDD 09](09_seguranca.md#endereços-do-kernel-no-log), pagos no #14); sem canal, o processo morre do mesmo
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
| `CPUID` em anel 3 | **Executa em qualquer anel** (**verificado**, espelho do SDM, [cpuid](https://www.felixcloutier.com/x86/cpuid): "can be executed at any privilege level"; a lista de exceções não tem `#GP` por CPL). O Linux sabe **trapear** o `CPUID` de usuário pelo *CPUID faulting* (`MSR_MISC_FEATURES_ENABLES`, `0x140`, bit 0; `ARCH_SET_CPUID` em `arch_prctl`: **verificado no código do Linux**); o **texto do SDM** sobre isso **NÃO VERIFICADO**. O RustOS não liga o *faulting* na fase 4 |
| `XGETBV` em anel 3 | Funciona e devolve o `XCR0` que o kernel ligou (x87, SSE, AVX, AVX-512 se a CPU tem, PKRU); a doc do kernel diz para "consult CPUID and use XGETBV to evaluate which features have been enabled by the kernel" (pesquisa `fpu-simd.md`) |
| `AT_HWCAP` | 0 na fase 4 (decisão futura da fase 5) |
| AMX | **Fora** do `XCR0` ([SDD 08](08_roadmap.md)); instrução de AMX dá `#UD` e o processo morre. O pedido de AMX só é preciso com sinais (sigframe), que a fase 4 não tem |
| SVE e SME | `CPACR_EL1.ZEN`/`SMEN` seguem zero: acesso trapeia (`EC` `0x19`, `0x1D`) e o processo morre. [DEBT-0035](../tech-debt/0035-sve-sme-e-amx-sem-estado-por-tarefa.md) segue aberta, com o gatilho reescrito para o primeiro programa que use SVE, SME ou AMX, ou o primeiro sinal Linux |
| `rdtsc` | `CR4.TSD = 0` já é o padrão. A opção por processo (`PR_SET_TSC`) **continua decisão futura**; a fase 4 não a oferece |
| Teste | `user/` roda duas threads em processos diferentes, cada uma põe um valor em `xmm0`/`v0` e o confere em laço. O `asm!` está num crate de `arch/` ([onde mora](#onde-cada-peça-mora-no-hexágono)) |

## Onde cada peça mora no hexágono

### Crates novos

**Crate novo em `core/` e pasta nova de primeiro nível são o caso 1**; a regra 5 do guidelines
03 que muda é o caso 3. As decisões [7](#decisão-7--crates-novos-em-core) e [8](#decisão-8--pasta-user-e-a-regra-5-do-guidelines-03) as aprovaram.

| Crate | Pasta | O que faz | `unsafe` | Depende de |
| --- | --- | --- | --- | --- |
| `rustos-elf` **(novo; ligado ao `kernel/` na 4b)** | `core/elf` | Validação e leitura de ELF64: `&[u8]` para `Image { entry, loads, phdr }`; o chamador passa `Limits` (ISA e faixa do usuário) | proibido | nada |
| `rustos-cpio` **(novo; ligado ao `kernel/` na 4b)** | `core/cpio` | Leitor `newc`, iterador sem alocação | proibido | nada |
| `rustos-object` **(novo, feito na 4c, parte pura)** | `core/object` | `HandleTable<T, S>`, regras de direito, duplicar, substituir e transferir, contagem de handles, fila do canal, sinais ([seção](#o-que-o-coreobject-torna-verdade-fatia-4c-parte-pura)) | proibido | `rustos-hal` |
| ABI (números, `Status`, direitos, constantes de layout, `ProcessEvent`, mensagem de boot) | `core/hal/src/abi.rs` (**módulo, não crate**) | O contrato com o programa de usuário, a "primeira porta de entrada" do [SDD 02](02_arquitetura.md) | proibido | nada |
| `rustos-user-rt` **(novo, feito na 4b)** | `user/rt` | Runtime dos programas: ponto de entrada em Rust, `panic`, alocador mínimo, leitor de auxv, carregador de ELF de usuário | **mínimo, com `// SAFETY:`**: `GlobalAlloc`, a leitura do auxv e `#[unsafe(no_mangle)]` o exigem; nenhum `asm!` | `hal` (a ABI), `elf`, `cpio`, `arch-<isa>-user` |
| `rustos-user-init` e programas de teste **(novos; na 4b, `user/init` e `user/tests/big`)** | `user/init`, `user/tests/*` | O `init`, o teste de falha, o de SIMD, o de syscall nula | proibido no `init` (`forbid(unsafe_code)`; o `unsafe` fica no `user/rt`). Em `user/tests/*`, `deny(unsafe_code)` com `#[allow]` pontual e `// SAFETY:` só para syscall crua em teste hostil ([guidelines 03](../guidelines/03_camadas_e_dependencias.md), regra 11) | `user-rt` |
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
| Árvore de regiões do VMAR, alocação com ASLR, bookkeeping de mapeamentos | `core/mm/src/vmar.rs` (**feito**, [seção](#a-árvore-de-regiões-coremmsrcvmarrs)) |
| Gerador de ASID/PCID | `core/mm/src/asid.rs` (**feito**, [seção](#o-gerador-de-pcid-e-asid-coremmsrcasidrs)) |
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
| `MAX_TASKS` e `Composite<4, 32>` por CPU | Sobem (96 bytes por tarefa justa **em cada CPU**; [SDD 15](15_tarefas_e_preempcao.md)); a capacidade vira configurável ou cresce do heap. O número entra no PR |
| Pilha de kernel por thread de usuário | 4 páginas (16 KiB) como as de hoje, fixadas pelo pico medido no #19 ([SDD 15](15_tarefas_e_preempcao.md#pico-de-pilha)); a 4a remede com syscall e falta de usuário ([DEBT-0070](../tech-debt/0070-pilha-de-16-kib-sem-syscall-nem-falta-de-usuario.md)); a janela de pilhas passa a reciclar endereço (lista livre), senão laço de criar e matar esgota a janela de 1 GiB |
| Troca de contexto | Se o processo muda, troca `CR3`/`TTBR0` **desta CPU** (com PCID/ASID e a regra de geração), atualiza `active_cpus` e escreve `kernel_rsp` e o `rsp[0]` da TSS **desta CPU** |
| Contas de tempo | O tempo de CPU por tarefa já existe; processo soma as threads |
| `ps` | Uma coluna de processo; a de SIMD já existe |

### Como o xtask compila e empacota, sem crate externo

| Passo | Comando | O quê |
| --- | --- | --- |
| 1 | `cargo xtask user --arch A [--profile P]` (**feito na 4b**, `xtask/src/user.rs`; compila `init` e `big`) | Compila cada crate de `user/` com a lista inteira de flags de usuário em `CARGO_ENCODED_RUSTFLAGS` (as do `config.toml` **não** somam: [linker e alvo](#o-que-muda-no-linker-e-no-alvo)) e `--target-dir target/user/<isa>` |
| 2 | `cargo xtask initramfs --arch A [--out caminho] [nome=]arquivo...` | Escreve `target/initramfs/<isa>.cpio` em `newc`, com um escritor em Rust puro em `xtask/src/initramfs.rs` (o `xtask` não depende de crate do workspace, regra 6, então não reaproveita o `core/cpio`), sem o binário `cpio` do host. Recebe `(nome, bytes, modo)`; a linha de comando põe o modo `0755`. Grava `070701`, os 13 campos em 8 dígitos hexadecimais, o nome com NUL, os dados e o `TRAILER!!!`; nome e dados alinham a 4 bytes a partir do início do arquivo, que não é preenchido até 512. Campos fixos: `c_ino` sequencial a partir de 1, `c_mtime`, `uid`, `gid`, `dev`, `rdev` e `c_chksum` em 0, `c_nlink` 1; a mesma entrada dá o mesmo arquivo, byte a byte. Recusa nome vazio, com NUL, repetido ou com componente vazio, `.` ou `..` (isso pega `/` inicial, `./` e `/` final), o nome `TRAILER!!!` inteiro (o leitor o toma como o fim; `a/TRAILER!!!` vale), nome de 4096 bytes ou mais, dado de 4 GiB ou mais, 2^32 arquivos ou mais e modo com bit fora de `0o7777`. **Não gera diretórios**: o leitor compara o nome inteiro (`bin/sh` é um nome) e o `init` é plano. Testado contra bytes de ouro escritos à mão, todos os restos de nome módulo 4, arquivo vazio e grande, e, só em desenvolvimento, contra o `bsdcpio` (`cpio -it` e `cpio -idv` listam e extraem os mesmos bytes) |
| 3 | `cargo xtask image` | **Feito (4b).** Compila os programas (o `init` e, se o kernel tem `selftest`, o `big`), gera `target/initramfs/<isa>.image.cpio` (`initramfs::write_image`), o copia para `boot/initramfs.cpio` na ESP e acrescenta ao `limine.conf` `module_path: boot():/boot/initramfs.cpio` e `module_string: initramfs`. O kernel não depende dos programas |
| 4 | `make smoke` | Exige as linhas novas ([abaixo](#linhas-que-o-smoke-passa-a-exigir)) |

A ordem de build é usuário, initramfs, imagem; o kernel **não** depende dos programas de
usuário, então mexer no `init` não recompila o kernel.

### Linhas que o smoke passa a exigir

Em inglês, como toda saída serial. As da 4a e da 4b valem hoje, no modo de teste; o boot normal as proíbe (o `init` só roda no modo de teste, [DEBT-0087](../tech-debt/0087-init-so-no-boot-de-teste.md)), na sessão do M1 só as três de lançar processo são permitidas; o smoke digita a sessão depois do prompt, nos dois modos.

| Linha | Fatia |
| --- | --- |
| `[rustos] user: entered ring3/el0` (a primeira syscall do boot; o texto é o mesmo nas duas ISAs) | 4a |
| `[rustos] user: fault contained (pid N)` | 4a |
| `[rustos] user self-test: ok` (os casos acima) | 4a |
| `[rustos] user: init loaded (N segments)`, `[rustos] user: pid 1 exited (code 0,` e, do `init`, `[user] init: ok` | 4b |
| `[user] init: boot channel, console and handles ok` e `[user] handles: ok (` (os casos hostis); `[rustos] initramfs: 10 files,` (o `init`, as quatro demonstrações e os cinco de teste) | 4c, parte 1 |
| `[user] lazy: ok (` e `[rustos] user: demand paging: ` (a pilha e o `.bss` sob demanda, a página zero e a cópia dela) | 4c, paginação sob demanda (**feito**) |
| `[user] child: hello from a child process`, `[user] init: the child process ran and terminated`, `[user] procs: ok (`, `[rustos] user: vmo cases: ok` e `[rustos] user: processes left 0 frames, 0 B of heap and 0 promised pages` | 4c, parte 3 (**feito**) |
| `[rustos] tlb shootdown: ok (cpu` (o teste entre CPUs; na máquina de uma CPU só sai `skipped, one cpu`, que o smoke recusa) | 4c, *shootdown* (**feito**) |
| Depois do prompt, nos dois modos: `[user] hello from a user process (`, `exit code 0 (pid`, `[user] sysinfo: up `, `[user] crash: going down the stack`, `[rustos] user: fault contained (pid`, `killed: page fault`, `started pid` e `[user] count: done` | M1 (**feito**) |
| `[rustos] mitigations: ` (nos dois modos) | 4e, parte 1 (**feito**) |
| `[rustos] cpu features: ` (só o modo de teste) | 4e, parte 1 (**feito**) |
| `[rustos] user: self-test ok` (handles, SIMD, falha, canal) | 4d |

## Medidas da fase

Regra do projeto: o mesmo comando antes e depois, nas duas ISAs ([SDD 07](07_desempenho_e_recursos.md)). Cada número entra no SDD 07 com a máquina e o acelerador.

| Medida | Como (proposta) |
| --- | --- |
| **Syscall nula** | `user/tests/null_syscall`: 1 M de `nop`, tempo por `rdtsc`/`cntvct`, mediana de 5, aquecimento descartado |
| **Entrada com KPTI** | O mesmo laço com `kpti=on` e `off` |
| **Criação de processo** | `process_create`, VMOs e mapeamentos de um programa mínimo, `thread_create`, `process_exit` e espera do `TERMINATED`, em laço |
| **Falta de página de usuário** | Três laços sobre 64 MiB: primeiro toque de escrita (zera um frame), leitura de página não tocada (página zero), escrita sobre a página zero (COW). Tempo por página. **Medida na 4c** (`user/tests/lazy`, três laços de 16 MiB; [SDD 07](07_desempenho_e_recursos.md#paginação-sob-demanda-e-shootdown-fatia-4c)) |
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

O zeramento do primeiro toque tem peso próprio. No aarch64 HVF, depois do `DC ZVA` (#11), o
4 KiB quente custa 18 a 55 ns (típico 20) e o par alocar e liberar, 102 a 133 ns ([SDD 07](07_desempenho_e_recursos.md));
no x86_64 o número só vale em KVM ou metal ([DEBT-0058](../tech-debt/0058-zerar-pagina-no-x86-sem-medida-em-hardware.md)).

**O que a QEMU não prova.** Sob TCG o TLB é de software e o custo de CR3, PCID e ASID não
representa o metal (**inferência**); KPTI e PCID só valem sob KVM, HVF (aarch64) ou metal
([DEBT-0014](../tech-debt/0014-x86-sem-salvaguardas-de-metal.md), [DEBT-0041](../tech-debt/0041-escalonador-do-x86-sem-medida-fora-do-tcg.md)). Os números do x86_64 sob TCG servem para
regressão entre PRs, não para comparar com o Linux.

## Dívidas que a fase paga, move ou abre

O #11 e o #13 pagaram as dívidas 0011, 0018, 0020, 0025, 0036 e 0039 (seus arquivos saíram de
`docs/tech-debt/`).

| Dívida | O que a fase faz |
| --- | --- |
| Dívida 0012, exceção de usuário só para a CPU | **Paga na 4a**: a origem se distingue pelo `CS` (x86_64) ou pelo grupo do vetor (aarch64), o processo morre e o kernel segue; o canal de eventos fecha na 4d |
| [DEBT-0043](../tech-debt/0043-simd-sem-trap-de-uso-indevido.md) SIMD sem trap | **Paga** na 4d: `simd: true` em toda thread de usuário |
| Dívida 0059, estado do ChaCha20 derramado na pilha | **Paga na 4a**: a volta ao usuário repinta a pilha do kernel abaixo do quadro, até 128 bytes de pilha limpa (a heurística é a [DEBT-0073](../tech-debt/0073-limpeza-da-pilha-so-ate-128-bytes-de-limpo.md)); custo medido no laço de syscall nula, com e sem a limpeza ([SDD 07](07_desempenho_e_recursos.md#syscall-nula-e-limpeza-da-pilha-fatia-4a)) |
| Dívida 0055 Kernel sem *shootdown* de TLB | **Paga na 4c**: vetor novo de IPI no x86_64 sobre o mecanismo que já existe, `tlbi ...is` no aarch64 ([seção](#shootdown-de-tlb-paga-a-dívida-0055-na-4c)); o que sobra (medida em KVM, faixa em lote) é a [DEBT-0122](../tech-debt/0122-shootdown-sem-medida-em-kvm-nem-lote.md). A regra de geração de PCID/ASID é da 4e |
| Dívida 0085 Pilha de usuário mapeada inteira | **Paga na 4c**: a pilha é região preguiçosa; o menor processo caiu de 32 para 17 frames ([SDD 07](07_desempenho_e_recursos.md#paginação-sob-demanda-e-shootdown-fatia-4c)) |
| [DEBT-0058](../tech-debt/0058-zerar-pagina-no-x86-sem-medida-em-hardware.md) Zerar página no x86 sem medida em hardware | O zerar entra no caminho da **falta de página de usuário**. A 4f mede a falta (primeiro toque, página zero, COW) nas duas ISAs; o do x86_64 só decide `stosq` contra `stosb` sob KVM ou metal (o gatilho da dívida): sob TCG a 4f registra o número e **deixa a dívida aberta** |
| [DEBT-0054](../tech-debt/0054-heap-com-lock-global.md) Heap com lock global | O gatilho dispara (4c, processos criados em N núcleos). Medir no `bench`; pagar se a disputa passar de 10% |
| [DEBT-0057](../tech-debt/0057-tsc-entre-nucleos-sem-verificacao.md) TSC entre núcleos | Não paga; o contrato de `clock_get_monotonic` diz "monotônico por CPU" no x86_64 até lá |
| [DEBT-0035](../tech-debt/0035-sve-sme-e-amx-sem-estado-por-tarefa.md) SVE, SME e AMX | O gatilho dispara, mas a fase **não liga** nada: o acesso trapeia e o processo morre. Reescrever o gatilho no PR da 4d |
| [DEBT-0019](../tech-debt/0019-aleatorio-sem-fonte-de-hardware-fraco.md) Aleatório fraco | O ASLR é o primeiro consumidor de segurança: em CPU sem fonte de hardware o ASLR é fraco, e o boot o diz |
| Dívida 0022, endereço do kernel no log (paga no #14) | O `ProcessEvent` e o `debug_write` seguem a mesma regra: nenhum endereço do kernel sai; o `Fault` leva só endereço de usuário |
| Dívida 0023, aarch64 sem `WXN` | **Paga na 4a**: o teste na QEMU confirmou o efeito sobre EL0 (uma página RWX de usuário deixa de executar com o `WXN`); a linha `protections:` do aarch64 ganhou `wxn` |
| Dívida 0037, pilha sem medida de pico (paga no #19: 16 KiB pelo pico medido) e [DEBT-0070](../tech-debt/0070-pilha-de-16-kib-sem-syscall-nem-falta-de-usuario.md) | A 4a remediu com syscall e falta fatal: 2,0 KiB e 1,6 KiB de 16 (`release`). Os 16 KiB ficam; a 0070 segue aberta até a 4c e a 4f medirem handles, canais e a falta resolvida |
| [DEBT-0052](../tech-debt/0052-afinidade-so-no-spawn.md) Afinidade só no `spawn` | Não dispara: a ABI nativa não tem afinidade |
| **Abertas pela 4a:** [DEBT-0073](../tech-debt/0073-limpeza-da-pilha-so-ate-128-bytes-de-limpo.md) (limpeza da pilha até 128 bytes limpos), [DEBT-0077](../tech-debt/0077-iretq-sem-fixup-de-falha-fora-do-rip-e-do-rsp.md) (`iretq` sem *fixup*), [DEBT-0078](../tech-debt/0078-aarch64-excecao-do-el1h-na-pilha-de-falhas.md) (EL1h na pilha de falhas); [DEBT-0081](../tech-debt/0081-debug-write-de-512-bytes-com-a-irq-mascarada.md) (`debug_write` com a IRQ mascarada) e [DEBT-0082](../tech-debt/0082-sem-lfence-depois-do-swapgs-condicional.md) (sem `lfence` depois do `swapgs`); [DEBT-0072](../tech-debt/0072-pilhas-de-excecao-sem-medida-permanente.md) agora inclui a IST4 | A 4a fecha os itens dela |
| **Pagas ou reescritas na parte 1 da 4c:** [DEBT-0070](../tech-debt/0070-pilha-de-16-kib-sem-syscall-nem-falta-de-usuario.md) (a pilha foi remedida com handles e canais: 4,8 KiB de 16 no pico das tarefas, 5,0 KiB na syscall; resta a falta resolvida); [DEBT-0090](../tech-debt/0090-ciclos-entre-filas-de-canais-nao-coletados.md) (a cobrança de 256 pontas existe); [DEBT-0091](../tech-debt/0091-fila-do-canal-com-armazenamento-inline.md) (agora com o custo do modelo do par); [DEBT-0087](../tech-debt/0087-init-so-no-boot-de-teste.md) (a primeira metade do gatilho, o canal de boot, ocorreu; a outra, o lançamento de processos, é da parte 3). **Abertas:** [DEBT-0111](../tech-debt/0111-espera-de-objeto-por-mascara-de-slots-de-tarefa.md) (as 0110 e 0112 foram pagas na parte 3). **Da parte 3:** pagas a 0110, a 0112, a 0120 e a 0123; reescritas a 0081, a 0087, a 0111, a 0121 e a 0122; abertas a [0140](../tech-debt/0140-abi-sem-tamanho-de-vmo.md), a [0141](../tech-debt/0141-vmo-sem-pagina-zero.md) e a [0142](../tech-debt/0142-objeto-do-kernel-sem-try-new.md).

**A registrar no PR de cada fatia:** mitigações de CPU sem teste fora da QEMU (4e; gatilho: o primeiro boot em metal); ciclos entre filas de canais não coletados ([DEBT-0090](../tech-debt/0090-ciclos-entre-filas-de-canais-nao-coletados.md), registrada com o `core/object`); imagem de usuário sem ASLR (4b; [decisão 5](#decisão-5--pie-de-usuário-agora-ou-depois), registrada: [DEBT-0083](../tech-debt/0083-imagem-de-usuario-sem-aslr.md)) | Cada uma com gatilho de evento, como manda o `AGENTS.md` |

**Pagas antes da fase:** 0011 (GIC e timer só na CPU 0), 0018 (frames sem cache por CPU), 0036
(estado de troca global) e 0039 (escalonador de uma CPU), no #13; 0020 (zerar página pelo `memset`) e
0025 (resíduo de chave na pilha), no #11. O resto do texto as cita no passado.

**O que fica de fora, e vira dívida com gatilho no PR em que o código nascer:** imagem
de usuário sem ASLR ([decisão 5](#decisão-5--pie-de-usuário-agora-ou-depois)), `vmar_protect`, sub-VMARs, TLS, `fork` e COW inteiro, multithread nativo
se a variante enxuta vencer, `rdtsc` por processo, AMX e VL do SVE por processo.

## Lacunas a fechar no PR da 4c

A revisão do desenho as deixou para o PR que codar os canais. Nenhuma muda uma decisão
do dono. Cada uma vira texto no SDD e teste no mesmo PR.

| # | Lacuna | O que o PR da 4c escreve e testa |
| --- | --- | --- |
| L1 | Entre a reserva do slot e o *commit* do `channel_write`, a ponta oposta pode morrer em outra CPU (SMP) | O *commit* reconfere a ponta viva sob o lock do canal; se morreu, os handles seguem a regra de descarte. **Feita** ([seção](#o-que-fechou-das-lacunas)) |
| L2 | A repetição do `channel_read` (sequência mudou) | A repetição libera as reservas do passo 2, e um valor de handle reservado e solto não reaparece. **Feita** |
| L3 | Regra de lock dos canais não está escrita (**escrita**, [acima](#ordem-de-lock-lacuna-l3)) | Um lock por par de pontas (A→B e B→A pegam o mesmo); a destruição tira as mensagens sob o lock e fecha os handles **depois** de soltá-lo; o passo 1 do `channel_read` valida `ch` sob a tabela antes do canal; ordem global tabela → canal |
| L4 | Chamadas que devolvem handle por ponteiro de saída (`channel_create`, `process_create`) | O mesmo padrão reserva → copia → *commit*: falha no `copy_to_user` desfaz a criação, sem objeto órfão. **Feita** para `channel_create` e `process_create` (parte 3) |
| L5 | `fault_kind` contra a tabela do `ProcessEvent` | Já concordam (`breakpoint` incluído); manter um teste que compare as duas |
| L6 | O `check-layers` não expressa "o `user/` depende só de `elf` e `cpio` entre os crates de `core/`", nem a isenção de `unsafe` só do `user/rt` | Lista por nome no `xtask`, com teste vermelho antes do verde (já pedido para a 4a, ver [guidelines 03](../guidelines/03_camadas_e_dependencias.md)) |

## Decisões e fontes

O dono aprovou as nove decisões abaixo no #9, em 2026-11-18, **todas como o desenho as
recomendava e sem mudança**. Cada uma é caso de parada do [`AGENTS.md`](../../AGENTS.md); o
texto deste capítulo já as aplica, e as tabelas de opções que as precederam saíram com o
Draft. As marcas de verificação são as de [Como ler as marcas](#como-ler-as-marcas).

### As decisões do dono

"Fatia" é a [fatia de código](#o-que-a-fase-entrega-por-fatia) que depende da decisão.

| # | Decisão | O que foi escolhido | Caso | Fatia |
| --- | --- | --- | --- | --- |
| 1 | Formato do executável nativo | ELF64 estático `ET_EXEC`; os handles iniciais vão no canal de boot, não no cabeçalho | 6 | 4b |
| 2 | ABI nativa: lista, números, nomes e contrato | 18 chamadas e um `nop`, em blocos por área; `duplicate` e `replace` separados; handle de 64 bits; experimental até a fase 5 | 6 | 4a |
| 3 | Bits de direito | 11 bits em `u32`, nomes e sentido do Zircon; sem `GET_PROPERTY`/`SET_PROPERTY`; `INSPECT` reservado; uid e gid fora | 6 | 4c |
| 4 | De onde vem o `init` | Um módulo do Limine com cpio `newc`; o kernel acha `init` | 6 | 4b |
| 5 | PIE de usuário | Depois: `ET_EXEC` na fase 4, PIE no início da fase 5 | 6 | 4b |
| 6 | Falha de usuário | Canal de eventos, sem sinal; o processo morre; o dono decidiu ciente do custo escondido | 6 | 4a |
| 7 | Crates novos em `core/` | `core/elf`, `core/cpio`, `core/object`; a ABI como módulo `abi` do `hal` | 1 | 4b |
| 8 | Pasta `user/` e a regra 5 do guidelines 03 | `user/` com os adaptadores `arch/<isa>-user`; regra 5 alterada, regras 11 e 12 novas | 1 e 3 | 4a |
| 9 | Overcommit na ABI nativa | Contabilidade estrita, por processo | 6 | 4c |

O que cada decisão **fixa além do que o capítulo já diz**, e o que ela muda no código:

#### Decisão 1 — formato do executável nativo

O formato está em [Executável e carregador](#executável-e-carregador). **O que muda:** `core/elf`
(decisão 7); `user/link/*.ld`; `kernel/src/loader.rs`.

#### Decisão 2 — ABI nativa: lista, números, nomes e contrato

A lista, a convenção e o contrato estão em [ABI nativa mínima](#abi-nativa-mínima). Respostas às
perguntas que a decisão fechou:

| Pergunta | Resposta |
| --- | --- |
| Numeração | Blocos por área (`0x00` sistema, `0x10` handles, `0x20` memória, `0x30` processos, `0x40` canais) |
| Retorno | Status em `rax`/`x0`, valor em `rdx`/`x1` |
| Quem cria processo | Handle do pai com `MANAGE_PROCESS`; `Job` só na fase 9 |
| `nop` de medida | Sempre presente, número `0x00` |
| Aleatório | `random_get(ptr, len)` ambiente, até 256 bytes: o `init` precisa dele para `AT_RANDOM` dos filhos |
| Autoridade sem handle | Só a lista de [Autoridade que não passa por handle](#autoridade-que-não-passa-por-handle) |
| Valores numéricos (`Status`, sinais, flags, direitos iniciais, limites do canal, `ProcessEvent`, mensagem de boot) | Os das tabelas do [contrato numérico](#contrato-numérico) |
| **Largura do handle e orçamento da geração** | **Handle de 64 bits** (geração de 48 bits, índice de 16), fixado **antes de congelar a ABI**. Com 32 bits, cada processo gastaria até cerca de 2^32 handles na vida e depois toda criação daria `NoMemory` (cerca de 71 minutos a 1 M de handles por segundo); a lista livre FIFO só distribuiria o gasto. Com 64 bits: `channel_create` e `process_create` devolvem o par por **ponteiro de saída** (dois `u64` não cabem em `rdx`/`x1`); os arrays de handles de `channel_write` e `channel_read` são `u64`; o slot sobe de 24 para 32 bytes (pior caso de 4 096: 128 KiB). `ProcessEvent` e a mensagem de boot não mudam de tamanho. A conta está na [tabela de handles](#tabela-de-handles) |
| Estabilidade | Experimental até o fim da fase 5 |

**O que muda:** `core/hal/src/abi.rs`; `kernel/src/syscall.rs`; `user/rt` (stubs); `arch/*-user` (`asm!`).

#### Decisão 3 — o conjunto inicial de bits de direito

Os 11 bits estão em [Direitos iniciais](#direitos-iniciais-decisão-3). Uid e gid não entram: a
tradução vive no adaptador Linux (fase 5). **O que muda:** `hal::abi::Rights`; `core/object`
(regras de subconjunto); testes de propriedade.

#### Decisão 4 — de onde vem o `init`

O `newc` é lido por ferramenta de fora, e o módulo é pedido novo ao Limine, que já é binário
aceito; por isso é caso 6. Está em [Origem](#origem-decisão-4). **O que muda:** pedido de módulo em
`boot/limine` e `BootInfo::modules()`; `core/cpio`; mapeamento do módulo em `kernel/src/vm.rs`;
`xtask/src/initramfs.rs`; `limine.conf`.

#### Decisão 5 — PIE de usuário: agora ou depois

Os números **verificados** (Linux `arch/x86/include/asm/elf.h` e `arch/arm64/include/asm/elf.h`):
`R_X86_64_RELATIVE` = 8, `R_AARCH64_RELATIVE` = 1027. O kernel já é PIE (888 relocações
relativas no x86_64, 648 no aarch64: [SDD 13](13_memoria_virtual.md)); o custo do `ET_DYN` é
código e teste nos dois carregadores, não pesquisa. **A abrir no PR da 4b:** `DEBT-NNNN`
"imagem de usuário sem ASLR", gatilho: o primeiro programa que não seja nosso, ou o início da
fase 5. **O que muda:** `core/elf` recusa `ET_DYN`; `user/link/*.ld` fixa a base.

#### Decisão 6 — falha de usuário: canal de eventos ou sinal síncrono

**O custo escondido, que o dono aceitou.** O adaptador Linux da fase 5 precisa entregar
`SIGSEGV`, `SIGBUS`, `SIGILL` e `SIGFPE` com *handler* do programa. Isso exige **capturar a
falta sem matar o processo**: a thread faltosa fica suspensa, o adaptador lê e altera o
contexto (monta o sigframe) e a retoma. É mecanismo novo: estado "suspensa por exceção" na
thread, leitura e escrita do contexto de outra thread e uma chamada de retomada, e entra a
decisão de AMX e SVE que a escolha adia. **A escolha é reversível só com esse trabalho**: o
canal de eventos da fase 4 vira o início dele (a mensagem de `Fault` já leva o que o sinal
precisa), mas a fase 5 paga a suspensão e a retomada. O canal paga a
a dívida 0012, paga na 4a. **O que muda:**
`hal::Fault.from_user`; `hal::abi::ProcessEvent`; `kernel/src/process.rs`; o último argumento
de `process_create`.

#### Decisão 7 — crates novos em `core/`

`core/elf` e `core/cpio` não dependem de nada; `core/object` depende só do `hal`. A ABI é módulo do `hal`,
não crate (um `core/abi` quebraria a regra 2 do guidelines 03: caso 3). **O que muda:**
`Cargo.toml` (`[workspace]`, `[workspace.dependencies]`, `default-members`);
[SDD 03](03_mapa_de_codigo.md); testes no host (ELF e cpio inválidos, propriedade dos
direitos, entrelaçamento de handles).

#### Decisão 8 — pasta `user/` e a regra 5 do guidelines 03

O diff está feito em [`guidelines/03`](../guidelines/03_camadas_e_dependencias.md) (regra 5
alterada, regras 11 e 12 novas). **O PR da 4a traz o `check-layers` conhecendo `user/` e a
regra 12, com o teste vermelho antes do verde:** hoje `arch/x86_64-user` seria classificado
como `Adapter` e o `Kernel` poderia depender dele; `user/` não classifica em camada nenhuma.
**O que muda:** `Cargo.toml` (`default-members` sem `user/` nem `arch/*-user`, que só compilam
para os alvos bare-metal); `xtask/src/checks/layers.rs` (a camada `User`);
[SDD 03](03_mapa_de_codigo.md) e [SDD 05](05_build_e_execucao.md).

#### Decisão 9 — overcommit na ABI nativa

`vmo_create` e `vmar_map` reservam páginas contra o total; as páginas seguem preguiçosas. Falta
de memória vira `NoMemory` numa chamada, tratável, e não um processo morto na falta de página.
O custo (reservar mais que o uso real) **não foi medido**. **O limite que a decisão não
resolve:** um contador **global** estrito, sem limite **por processo**, deixa um processo
reservar a RAM inteira e negar `NoMemory` aos outros até os grupos de recursos da fase 9; a
fase 4 só impede que o total passe da RAM. A política é **por processo**: o adaptador Linux da
fase 5 poderá pedir o modo heurístico para os programas dele. O modo exato do Linux
(`vm.overcommit_memory`) **NÃO VERIFICADO**. **O que muda:** um contador de reserva no kernel;
`NoMemory` em `vmo_create` e `vmar_map`; o campo de política no Process.

### Decisões do agente

O `AGENTS.md` manda o agente decidir o que segue o desenho e as regras. Estas ficaram fora da
lista acima por não serem dos seis casos; cada uma diz o que a faria subir ao dono.

| Decisão | O que o agente decidiu | Contingência |
| --- | --- | --- |
| **KPTI** | Só em CPU vulnerável (detecção: `IA32_ARCH_CAPABILITIES.RDCL_NO`, bit 0, **verificado** em `msr-index.h`; mais a lista de CPUs imunes do Linux, **verificada** na tag `v6.12`), com chave de override `kpti=on\|off\|auto` na linha de comando do Limine (para medir as duas configurações; o nome é o do Linux, sem o prefixo `rustos.` das outras chaves). **Feito na parte 1:** a detecção, a lista (**verificada** na tag `v6.12`) e a chave, que só muda o relatório. Sem informação, vulnerável. O [SDD 08](08_roadmap.md) e o [SDD 09](09_seguranca.md) já dizem "só em CPU vulnerável" | Se o dono quiser sempre ligado, a 4e perde a detecção e o override; o resto não muda |
| **Detecção e linha de mitigação no `hal`** | `Vulns`, `KptiPolicy`, `Mitigations` e o clamp em `core/hal` (módulos `mitigation` e `nospec`); a decodificação por ISA em `arch/<isa>/src/mitigation.rs`, pura. Porta nova no `hal` é do agente ([`AGENTS.md`](../../AGENTS.md)) | — |
| **A linha `mitigations:` nos dois modos** | Mesmo critério da `protections:` (um gate a confere e ela avisa); custa uma linha e cerca de 1 ms no x86_64 em TCG (o CPUID por núcleo é emulado), nada no aarch64. Só o `cpu features:` é do modo de teste | Se a medida em UART física ([DEBT-0067](../tech-debt/0067-log-do-boot-normal-sem-medida-em-uart-real.md)) pesar, a linha passa para o `fetch` |
| **BHB do aarch64 em CPU fora das listas** | Conta como afetada (sem informação, vulnerável); o Linux só marca as das listas dele | Alarme falso em CPU nova até a [DEBT-0106](../tech-debt/0106-listas-de-cpu-do-linux-so-em-parte.md) |
| **A máscara do clamp em `arch/`, nas duas ISAs** | A sequência do Linux em `asm!`, no x86_64 (`cmp; sbb`) e no aarch64 (`cmp; sbc; csdb`, a `CSDB` incondicional como no Linux); o `hal` não tem `asm!` e guarda a versão pura | Custa 10 ns por syscall no HVF ([SDD 07](07_desempenho_e_recursos.md)): tirar exige ler o requisito no Arm ARM ([DEBT-0105](../tech-debt/0105-csdb-do-clamp-custa-10-ns-sem-o-requisito-lido.md)) |
| **Despacho por tabela indexada** | `kernel/src/syscall.rs` indexa `TABLE` pelo número em vez de percorrer `Syscall::ALL`; é o que dá ao clamp um índice de verdade | — |
| **Framebuffer e entrada** | Ficam na fase 5, onde o [SDD 08](08_roadmap.md#decisões-futuras-casos-do-agentsmd) já os lista. O desenho deixa o caminho (VMO físico com política de cache no `vmar_map`; teclado por canal) | A interface é caso 6 quando chegar; nada a decidir na fase 4 |
| **Alvo de compilação dos programas** | Os mesmos alvos `*-unknown-none`, com `-C code-model=small` (x86_64) e `-C relocation-model=static`, linker script próprio e `--target-dir` separado ([SDD 05](05_build_e_execucao.md): sem nightly nem `build-std`). **Testado** com `rustc 1.100.0` nos dois alvos: compila e liga em `0x40_0000` ([o linker e o alvo](#o-que-muda-no-linker-e-no-alvo)). **Mudança de plano:** as quatro fontes de `rustflags` do Cargo são **exclusivas**, então o xtask passa a lista inteira por `CARGO_ENCODED_RUSTFLAGS` e as flags do `.cargo/config.toml` (o `pie` do kernel) não somam | O risco que sobra é `code-model` em programa maior (o teste mínimo não distingue `small` de `kernel`). Se um alvo JSON virar necessário, exige `build-std`: **volta ao dono** (ferramenta de build nova, caso 2) |
| **Init só no modo de teste (4b)** | Ver [A fatia 4b](#a-fatia-4b-como-entrou) e [DEBT-0087](../tech-debt/0087-init-so-no-boot-de-teste.md) | Se o dono quiser o `init` no boot normal já na 4b, é chamar `init::run` do boot e aceitar +10 ms no TCG do x86_64 |
| **`code-model=small` sem LTO no x86_64 (4b)** | `-C lto=off` na lista de flags | Se o dono preferir o modelo `kernel` com LTO, tira-se as duas flags ([DEBT-0084](../tech-debt/0084-usuario-do-x86-sem-lto-por-causa-do-code-model.md)) |
| **`arch/<isa>-user`** | Crates de adaptador do lado do usuário (`asm!` de `syscall`/`svc`, de `_start`, de `xmm0`/`v0` do teste). Cabem nas regras 4 e 8 do guidelines 03 (adaptador, crate numa pasta existente) | — |
| **Árvore de regiões em vetor ordenado, de capacidade fixa** | `core/mm/src/vmar.rs` ([seção](#a-árvore-de-regiões-coremmsrcvmarrs)): vetor ordenado em armazenamento do chamador, 32 bytes por região, guarda como marca da região, `unmap` que apara e parte, `Placement::Lowest` e `Random` | Registrada como [DEBT-0075](../tech-debt/0075-vmar-com-capacidade-fixa-de-regioes.md): se o primeiro processo real passar de umas centenas de regiões, o `memmove` de `reserve` e `unmap` (1,1 µs com 4 096) pede uma árvore; o desenho muda por trás da mesma API |
| **ASID/PCID sem liberação individual** | `core/mm/src/asid.rs` ([seção](#o-gerador-de-pcid-e-asid-coremmsrcasidrs)): o índice só volta no esgotamento; `flush_pending` por CPU no lugar da `generation_seen` | Se a 4e medir esgotamento frequente, o remédio é largura maior ou menos PCID por processo, não `release`. O lock em toda troca é a [DEBT-0076](../tech-debt/0076-asid-sem-caminho-rapido-sem-lock.md) |
| **ABI no `hal`** | Módulo `abi` em `core/hal` (a "porta de entrada" do [SDD 02](02_arquitetura.md)): números, `Status`, direitos, mensagens, `USER_TOP`. Porta nova no `hal` é do agente | Um crate `core/abi` é a opção A' da decisão 7 |

### O diff dos guidelines

[`guidelines/03`](../guidelines/03_camadas_e_dependencias.md) traz as regras alteradas e novas
(regra 5; regras 11 e 12 para `user/`, `arch/<isa>-user` e o `unsafe` mínimo do `user/rt`).
As outras decisões não tocam o guidelines. O `make layers-check` só aprende a camada `User` no
PR da 4a.

### Fontes lidas e o que ficou por ler

O relatório de conferência (URL e trecho literal de cada item) está na pesquisa
`phase4-verify.md`.

| Fonte | Usada para | Estado |
| --- | --- | --- |
| Intel SDM via [felixcloutier](https://www.felixcloutier.com/x86/sysret): `syscall`, `sysret`, [`iret`](https://www.felixcloutier.com/x86/iret:iretd:iretq), [`mov cr3`](https://www.felixcloutier.com/x86/mov-1), [`invpcid`](https://www.felixcloutier.com/x86/invpcid), [`cpuid`](https://www.felixcloutier.com/x86/cpuid) | Registradores; `STAR`; `IRETQ` (`NT` atual dá `#GP`, `RSP` não canônico dá `#SS`, bits restaurados); PCID e `INVPCID`; `CPUID` em qualquer anel | Lidas (espelho; o PDF do SDM não baixou) |
| [Xen XSA-7](https://xenbits.xen.org/xsa/advisory-7.html) | CVE-2012-0217 | Lida |
| Linux x86: [`entry_64.S`](https://github.com/torvalds/linux/blob/master/arch/x86/entry/entry_64.S), [`cpu/common.c`](https://github.com/torvalds/linux/blob/master/arch/x86/kernel/cpu/common.c), [`idt.c`](https://github.com/torvalds/linux/blob/master/arch/x86/kernel/idt.c), [`syscall_64.c`](https://github.com/torvalds/linux/blob/master/arch/x86/entry/syscall_64.c), [`msr-index.h`](https://github.com/torvalds/linux/blob/master/arch/x86/include/asm/msr-index.h), `processor-flags.h`, `process.c`, `nospec.h`, `nospec-branch.h` | Entrada, `FMASK`, `STAR`, `SYSRET` contra `IRET`, MSRs, `PCIDE`, Meltdown, `#DB` com IST, `fixup_bad_iret`, *CPUID faulting*, `array_index_nospec` | Lidos |
| Linux `v6.12` (tag): `arch/x86/kernel/cpu/common.c` (`cpu_vuln_whitelist`, `cpu_set_bug_bits`), `arch/x86/include/asm/{cpufeatures,msr-index,intel-family,barrier}.h`, `arch/x86/lib/cpu.c` (`x86_family`, `x86_model`), `include/linux/nospec.h` | Regras das vulnerabilidades, bits de CPUID e de MSR, família e modelo, `array_index_nospec` | Lidos na tag (a leitura de hoje, 2026-11-18) |
| Linux `v6.12`: `arch/arm64/kernel/{cpufeature,proton-pack}.c`, `arch/arm64/include/asm/{cpufeature,cputype,barrier}.h`, `arch/arm64/tools/sysreg` | `unmap_kernel_at_el0`, listas de MIDR, `supports_csv2p3`, `supports_clearbhb`, `CSDB`, posição de `CSV2`, `CSV3`, `SSBS`, `ECBHB`, `CLRBHB` | Lidos na tag |
| [`df.lth.se/~getz/ARM/SysReg/`](https://df.lth.se/~getz/ARM/SysReg/) (espelho do XML da Arm): `id_aa64pfr0_el1`, `id_aa64pfr1_el1`, `id_aa64mmfr1_el1`, `id_aa64isar2_el1`, `midr_el1` | Confirmação dos campos acima | Lido (secundária) |
| [InstLatx64](https://github.com/InstLatx64/InstLatx64) (commit `17be9a9`), QEMU [`target/arm/tcg/cpu64.c`](https://github.com/qemu/qemu/blob/v10.0.0/target/arm/tcg/cpu64.c) (`v10.0.0`) | Valores de CPUID, MSR e registradores de CPUs reais, para os testes | Lidos (secundária) |
| Linux arm64: [`entry.S`](https://github.com/torvalds/linux/blob/master/arch/arm64/kernel/entry.S), [`esr.h`](https://sources.debian.org/src/linux/6.12.69-1/arch/arm64/include/asm/esr.h), `arm_arch_timer.[ch]`, `tools/sysreg`, `pgtable-hwdef.h`, `uaccess.h` | Vetores, `sp_el0`, KPTI por `TTBR1`, `EC`, campos de `TCR_EL1`, `SCTLR_EL1`, `ID_AA64MMFR*`, bits de PTE, `CNTKCTL` | Lidos (**é o Linux, não a ARM ARM**) |
| Linux [`mm/memory.c`](https://github.com/torvalds/linux/blob/master/mm/memory.c) | Página zero na leitura, COW (`do_anonymous_page`, `wp_page_copy`) | Lido nesta rodada |
| Linux `spectre.rst`, `mds.rst` ([hw-vuln](https://github.com/torvalds/linux/tree/master/Documentation/admin-guide/hw-vuln)) | IBPB condicional, eIBRS preferido a retpoline, limpeza de buffers no MDS | Lidos nesta rodada |
| [syscall(2)](https://man7.org/linux/man-pages/man2/syscall.2.html), [`auxvec.h`](https://github.com/torvalds/linux/blob/master/include/uapi/linux/auxvec.h), [`elf-em.h`](https://github.com/torvalds/linux/blob/master/include/uapi/linux/elf-em.h), `elf.h` das duas ISAs, [inode(7)](https://man7.org/linux/man-pages/man7/inode.7.html) | Convenção; `AT_*`, `e_machine`, `R_*_RELATIVE`; `S_IFMT` | Lidos |
| [gABI](https://www.sco.com/developers/gabi/latest/ch5.pheader.html) (`ch4`, `ch5`) | `EI_*`, congruência de `p_vaddr` e `p_offset` **módulo o tamanho da página** | Lida nesta rodada |
| [x86-64 psABI](https://github.com/hjl-tools/x86-psABI), [AAPCS64](https://github.com/ARM-software/abi-aa) | `rsp` alinhado a 16 na entrada, layout de `argc`/`argv`/`envp`/auxv; `sp mod 16 = 0` | Lidos |
| Limine: `CONFIG.md` da `v12.10.0` e [`limine-protocol`](https://github.com/Limine-Bootloader/limine-protocol) (`trunk`) | `module_path`, `module_cmdline`; `LIMINE_MODULE_REQUEST_ID`, `string`, `tftp_ipv4` | Lidos; o `bootstrap` da `v12.10.0` fixa o `limine-protocol` em `877c2d4`, o topo do `trunk` em 2026-11-18, e o `limine.c` da etiqueta confere pedido e resposta (acima) |
| Cargo `config.md`, fonte do `rustc` (`targets/`, `spec/mod.rs`) | Quatro fontes exclusivas de `rustflags`; `code_model` e `relocation_model` dos alvos | Lidos **e testados** com `rustc 1.100.0` e `cargo build -v` |
| [Linux PTI](https://docs.kernel.org/arch/x86/pti.html), [entry_64](https://docs.kernel.org/arch/x86/entry_64.html), [x86_64 mm](https://docs.kernel.org/arch/x86/x86_64/mm.html), [arm64 memory](https://docs.kernel.org/arch/arm64/memory.html) | Trampolim, `swapgs`, layout | Lidos |
| Zircon: [rights](https://fuchsia.dev/fuchsia-src/concepts/kernel/rights), [handle_replace](https://fuchsia.dev/fuchsia-src/reference/syscalls/handle_replace), [channel_write](https://fuchsia.dev/fuchsia-src/reference/syscalls/channel_write), [channel_read](https://fuchsia.dev/fuchsia-src/reference/syscalls/channel_read), [process_create](https://fuchsia.dev/fuchsia-src/reference/syscalls/process_create), [process_start](https://fuchsia.dev/fuchsia-src/reference/syscalls/process_start), [channel_create](https://fuchsia.dev/fuchsia-src/reference/syscalls/channel_create), [conceitos](https://fuchsia.dev/fuchsia-src/concepts/kernel/concepts) | Direitos, atomicidade, descarte na ponta fechada, `arg1` fechado em erro, limites do canal | Lidas |
| [API do seL4](https://docs.sel4.systems/projects/sel4/api-doc.html), [Redox, schemes](https://doc.redox-os.org/book/ch05-06-scheme-operation.html) | Contraste | Lidas (direitos do seL4 não; o namespace do Redox está em mudança) |
| [cpio `newc`](https://docs.kernel.org/driver-api/early-userspace/buffer-format.html) | Gramática, `c_mode`, `c_namesize`, hard link, trailer | Lida |
| [mmap(2)](https://man7.org/linux/man-pages/man2/mmap.2.html), [signal(7)](https://man7.org/linux/man-pages/man7/signal.7.html), [getauxval(3)](https://man7.org/linux/man-pages/man3/getauxval.3.html), [binfmt_elf.c](https://github.com/torvalds/linux/blob/master/fs/binfmt_elf.c), [Hansen, lkml](https://lkml.iu.edu/1712.0/03977.html) | Guarda de pilha, `MAP_FIXED_NOREPLACE`, sinais, auxv, validação de ELF, custo de TLB | Lidos |

#### Por ler antes de codar

Só o que sobrou; cada item é pré-requisito da fatia que o usa.

| Item | Fatia |
| --- | --- |
| **ARM ARM** (as páginas do developer.arm.com não renderizam sem JavaScript; os campos de registrador acima vêm do Linux): o `imm16` do SVC em `ISS[15:0]`; a tabela de `AP[2:1]` por nível de exceção; um `ERET` com estado ilegal; a versão e o efeito do `UAO`; o nome `EL0VCTEN`. **Feitos na 4a, medidos na QEMU, não na ARM ARM:** a regra do PAN com `LDTR`/`STTR` e o efeito do `WXN` sobre EL0 ([A fatia 4a](#a-fatia-4a-como-entrou)) | 4a, 4e |
| **Intel SDM** em PDF (só o espelho foi lido): o texto sobre *CPUID faulting* (`MSR` `0x140`); se a entrada de exceção limpa `NT` (o `IRETQ` do kernel com `NT = 1` dá `#GP`) | 4a |
| Layout inicial da pilha do processo no aarch64, em fonte que não seja o `binfmt_elf.c` do Linux (a pesquisa de 2026-11-18: o sysvabi64 da Arm **não trata** da pilha inicial; glibc e musl só leem `argc` e `argv` e se apoiam no Linux) | 4b (**segue NÃO VERIFICADO**; o RustOS usa a ordem do Linux) |
| `-C code-model=small` com um programa de usuário **maior** que o mínimo (o teste não distingue `small` de `kernel`) | 4b (**feito**: o `big` roda nas duas ISAs; com LTO o `small` não liga, [DEBT-0084](../tech-debt/0084-usuario-do-x86-sem-lto-por-causa-do-code-model.md)) |
| Custo de IBPB, eIBRS, limpeza de buffers e KPTI: medir (o do `array_index_nospec` está no [SDD 07](07_desempenho_e_recursos.md)); STIBP e SSBD: ler as páginas do Linux; o texto do **SDM** e do **ARM ARM** sobre os bits da detecção e o requisito da `CSDB` ([DEBT-0105](../tech-debt/0105-csdb-do-clamp-custa-10-ns-sem-o-requisito-lido.md)) | 4e |

Levantamento de novembro de 2026.
