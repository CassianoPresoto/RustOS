# SDD 18 — Fase 4: decisões pedidas ao dono (Draft)

> **O que este capítulo responde:** quais decisões do desenho da fase 4
> ([SDD 17](17_processos_isolamento_e_syscalls.md)) são do dono, com as opções e a
> recomendação de cada uma; quais o agente já tomou, para o dono vetar; e de que fontes
> o desenho saiu.

**Estado: Draft.** Este capítulo existe só enquanto a pergunta está aberta. O merge do
Draft é a aprovação ([`AGENTS.md`](../../AGENTS.md), "Decisões"); depois dele, o que foi
decidido vira texto do SDD 17 e do guidelines, e este arquivo sai. Marcas de verificação:
as do [SDD 17](17_processos_isolamento_e_syscalls.md#como-ler-as-marcas).

## As decisões do dono

Nove, todas nos casos de parada do `AGENTS.md`. A **recomendação** é de quem escreveu o
desenho; vale o que o dono decidir. "Bloqueia" é a fatia de código ([plano](17_processos_isolamento_e_syscalls.md#o-que-a-fase-entrega-por-fatia-proposta)) que
não começa sem a resposta.

| # | Decisão | Caso | Recomendação | Bloqueia |
| --- | --- | --- | --- | --- |
| 1 | Formato do executável nativo | 6 | ELF64 estático `ET_EXEC` | 4b |
| 2 | ABI nativa: lista, números, nomes e contrato | 6 | 18 chamadas e um `nop`, em blocos por área; `duplicate` e `replace` separados; experimental até a fase 5 | 4a |
| 3 | Bits de direito | 6 | 11 bits em `u32`, estilo Zircon | 4c |
| 4 | De onde vem o `init` | 6 | Um módulo do Limine com cpio `newc` | 4b |
| 5 | PIE de usuário | 6 | Depois (início da fase 5) | 4b |
| 6 | Falha de usuário | 6 | Canal de eventos, sem sinal, **ciente do custo escondido** | 4a |
| 7 | Crates novos em `core/` | 1 | `core/elf`, `core/cpio`, `core/object` | 4b |
| 8 | Pasta `user/` e a regra 5 do guidelines 03 | 1 e 3 | Aprovar; o diff está em `docs/guidelines/03` | 4a |
| 9 | Overcommit na ABI nativa | 6 | Contabilidade estrita, por processo | 4c |

### Decisão 1 — formato do executável nativo

**Caso 6** (formato de executável: contrato que o mundo de fora enxerga).

| Opção | Prós | Contras |
| --- | --- | --- |
| **A. ELF64 estático `ET_EXEC`** | `rustc` e LLD já geram; `readelf` e depuradores funcionam; um só `core/elf` serve ao `init` e ao carregador de usuário; a fase 5 reaproveita | Herda o que o ELF cobre (realocação, TLS, interpretador): mitigado recusando o que a fase não usa; mais superfície de validação que um formato fixo |
| B. ELF `ET_DYN` estático (PIE) | ASLR da imagem | Exige aplicar `R_*_RELATIVE` ([decisão 5](#decisão-5--pie-de-usuário-agora-ou-depois)) |
| C. Formato próprio | Parser minúsculo; handles iniciais no cabeçalho | Linker script e alvo próprios; toda ferramenta é nossa; a fase 5 precisa do ELF mesmo |

**Recomendação:** A. Os handles iniciais vão no canal de boot, não no cabeçalho.

**O que muda:** `core/elf` ([decisão 7](#decisão-7--crates-novos-em-core)); `user/link/*.ld`; `kernel/src/loader.rs`.

### Decisão 2 — ABI nativa: lista, números, nomes e contrato

**Caso 6** (syscall). A lista, a convenção e o contrato de entrada estão no [SDD 17](17_processos_isolamento_e_syscalls.md#abi-nativa-mínima); aqui vão as opções.

| Opção | Prós | Contras |
| --- | --- | --- |
| **A. 18 chamadas e `nop`, por handle, `duplicate` e `replace` separados** | É o modelo A; cada chamada valida um direito; o Zircon (páginas lidas) separa as duas | Passa do "11 a 15" da estimativa; duas chamadas de handle quase iguais |
| A'. Variante enxuta de 16 (sem `thread_create`, `thread_exit`, `process_kill`; com `process_start`) | Menos superfície | Sem multithread nativo; o pai só mata o filho por canal |
| A''. `duplicate` e `replace` fundidos (flag `REPLACE`) | Uma a menos | Direitos exigidos diferentes numa só |
| B. Chamadas por caminho (schemes do Redox) | Uniforme com arquivo | Pede espaço de nomes e autoridade ambiente: contradiz o modelo A |
| C. Numeração do Linux | Programa Linux roda sem adaptador | É a fase 5; a ABI nativa não herda 500 números |

**Respostas recomendadas às perguntas que a decisão fecha:**

| Pergunta | Recomendação |
| --- | --- |
| Numeração | Blocos por área (`0x00` sistema, `0x10` handles, `0x20` memória, `0x30` processos, `0x40` canais) |
| Retorno | Status em `rax`/`x0`, valor em `rdx`/`x1` |
| Quem cria processo | Handle do pai com `MANAGE_PROCESS`; `Job` só na fase 9 |
| `nop` de medida | Sempre presente, número `0x00` |
| Aleatório | `random_get(ptr, len)` ambiente, até 256 bytes: o `init` precisa dele para `AT_RANDOM` dos filhos |
| Autoridade sem handle | Só a lista de [Autoridade que não passa por handle](17_processos_isolamento_e_syscalls.md#autoridade-que-não-passa-por-handle) |
| Valores numéricos (`Status`, sinais, flags, direitos iniciais, limites do canal, `ProcessEvent`, mensagem de boot) | Os das tabelas do [SDD 17](17_processos_isolamento_e_syscalls.md#contrato-numérico) |
| **Largura do handle e orçamento da geração** | **Handle de 64 bits** (geração de 48 bits, índice de 16), decidido **antes de congelar a ABI**. Com 32 bits, cada processo gasta até cerca de 2^32 handles na vida e depois toda criação dá `NoMemory` (cerca de 71 minutos a 1 M de handles por segundo); a lista livre FIFO só distribui o gasto, não o aumenta. Com 64 bits: `channel_create` e `process_create` devolvem o par por **ponteiro de saída** (dois `u64` não cabem em `rdx`/`x1`); os arrays de handles de `channel_write` e `channel_read` passam a `u64`; o slot sobe de 24 para 32 bytes (pior caso de 4 096: 128 KiB). `ProcessEvent` e a mensagem de boot não mudam de tamanho. O [SDD 17](17_processos_isolamento_e_syscalls.md#tabela-de-handles) está escrito com 32 bits (a base); a escolha reescreve só esses pontos |
| Estabilidade | Experimental até o fim da fase 5 |

**Recomendação:** A, com as respostas acima.

**O que muda:** `core/hal/src/abi.rs`; `kernel/src/syscall.rs`; `user/rt` (stubs); `arch/*-user` (`asm!`).

### Decisão 3 — o conjunto inicial de bits de direito

**Caso 6** (os direitos aparecem em toda chamada).

| Opção | Prós | Contras |
| --- | --- | --- |
| **A. 11 bits em `u32`, nomes e sentido do Zircon** | Conhecidos; `READ`/`WRITE`/`EXECUTE` valem como teto de mapeamento; 21 bits livres | `INSPECT` sem syscall na fase; o sentido varia por tipo de objeto |
| B. 6 bits (`DUPLICATE`, `TRANSFER`, `READ`, `WRITE`, `EXECUTE`, `MANAGE`) | Mais simples | `MANAGE` mistura criar processo, criar thread e matar; `MAP` some |
| C. Bits por tipo de objeto | Cada bit tem um sentido | Tabelas duplicadas; transferência entre tipos confusa |

**Recomendação:** A, sem `GET_PROPERTY`/`SET_PROPERTY` (sem propriedades na fase); `INSPECT` reservado. Uid e gid não entram: a tradução vive no adaptador Linux (fase 5).

**O que muda:** `hal::abi::Rights`; `core/object` (regras de subconjunto); testes de propriedade.

### Decisão 4 — de onde vem o `init`

**Caso 6** (o `newc` é lido por ferramenta de fora; módulo é pedido novo ao Limine, que já é binário aceito).

| Opção | Prós | Contras |
| --- | --- | --- |
| **A. Um módulo com cpio `newc`; o kernel acha `init`** | Um mecanismo; `newc` é o formato do initramfs do Linux (spec lida); o `init` lê os outros programas; a fase 5 usa o mesmo | Parser `newc` no caminho de confiança do kernel; o `init` fica duplicado na memória até devolver o módulo |
| B. Um módulo = só o ELF do `init` | Sem `core/cpio` | Só um programa; a fase 5 pede o cpio |
| C. Dois módulos: ELF do `init` e cpio | Kernel sem parser de cpio | Dois módulos; a fase 5 desempacota no kernel |
| D. Embutido na imagem (`include_bytes!`) | Sem pedido de módulo | Mexer no `init` recompila o kernel; bytes residentes para sempre |

**Recomendação:** A.

**O que muda:** pedido de módulo em `boot/limine` e `BootInfo::modules()`; `core/cpio`; mapeamento do módulo em `kernel/src/vm.rs`; `xtask/src/initramfs.rs`; `limine.conf`.

### Decisão 5 — PIE de usuário: agora ou depois

**Caso 6** (formato de executável).

| Opção | Prós | Contras |
| --- | --- | --- |
| **A. `ET_EXEC` na fase 4, PIE no início da fase 5** | Carregador menor; uma classe de erro a menos na 4b; o musl da fase 5 decide o formato com o programa à vista | Imagem do `init` em endereço fixo (só programas nossos) |
| B. `ET_DYN` com só `R_*_RELATIVE` | ASLR da imagem; o kernel já é PIE (888 relocações relativas no x86_64, 648 no aarch64: [SDD 13](13_memoria_virtual.md)) | Código novo nos dois carregadores; mais bits a validar |

Os números **verificados** nesta rodada (Linux `arch/x86/include/asm/elf.h` e `arch/arm64/include/asm/elf.h`): `R_X86_64_RELATIVE` = 8, `R_AARCH64_RELATIVE` = 1027. O custo de B é código e teste, não pesquisa.

**Recomendação:** A, e abrir `DEBT-NNNN` ("imagem de usuário sem ASLR"; gatilho: o primeiro programa que não seja nosso, ou o início da fase 5) no PR da 4b.

**O que muda:** `core/elf` recusa `ET_DYN`; `user/link/*.ld` fixa a base.

### Decisão 6 — falha de usuário: canal de eventos ou sinal síncrono

**Caso 6** (como o mundo de fora vê a falha de um processo).

| Opção | Prós | Contras |
| --- | --- | --- |
| **A. Canal de eventos; o processo morre** | Sem sigframe nem retomada: AMX e VL do SVE esperam a fase 5; o pai recebe `pc`, `sp` e endereço; paga a [DEBT-0012](../tech-debt/0012-excecao-de-usuario-so-para-a-cpu.md) | O programa não trata a própria falta; ver o custo escondido abaixo |
| B. Sinais síncronos POSIX na ABI nativa | Conhecido; permite tratar a falta | Exige sigframe, que depende do estado de SIMD (AMX, SVE): duas decisões antes de codar |
| C. Só matar | Zero ABI | O pai não sabe por quê |

**Custo escondido de A (para o dono decidir ciente).** O adaptador Linux da fase 5 precisa
entregar `SIGSEGV`, `SIGBUS`, `SIGILL` e `SIGFPE` com *handler* do programa. Isso exige
**capturar a falta sem matar o processo**: a thread faltosa fica suspensa, o adaptador
lê e altera o contexto (monta o sigframe) e a retoma. É mecanismo novo: estado "suspensa
por exceção" na thread, leitura e escrita do contexto de outra thread e uma chamada de
retomada, e entra a decisão de AMX e SVE que A adia. **A é reversível só com esse
trabalho**; o canal de eventos da fase 4 vira o início dele (a mensagem de `Fault` já leva
o que o sinal precisa), mas a fase 5 paga a suspensão e a retomada.

**Recomendação:** A. B só faz sentido se o dono quiser tratamento de falta já na fase 4.

**O que muda:** `hal::Fault.from_user`; `hal::abi::ProcessEvent`; `kernel/src/process.rs`; o último argumento de `process_create`.

### Decisão 7 — crates novos em `core/`

**Caso 1** (crate novo em `core/`).

| Opção | Prós | Contras |
| --- | --- | --- |
| **A. `core/elf`, `core/cpio`, `core/object`; a ABI como módulo `abi` do `hal`** | Cada crate faz uma coisa e roda no host; regra 2 do guidelines 03 intacta | Três crates para aprovar; `user/` compila o `hal` inteiro |
| A'. Como A, com a ABI num crate `core/abi` | `user/` depende só de constantes | Quebra a regra 2 ("`core/abi` é o único crate de `core/` de que os outros dependem"): caso 3 |
| B. `core/abi` e `core/exec` (elf e cpio juntos); objetos no `kernel/` | Menos crates | Handles e direitos sem teste no host; a regra 2 muda também |
| C. Nenhum: tudo no `kernel/` | Sem crate novo | Parser sem teste no host e sem `forbid(unsafe_code)`: contradiz o hexágono |

**Recomendação:** A. `core/elf` e `core/object` dependem só do `hal`; `core/cpio` de nada.

**O que muda:** `Cargo.toml` (`[workspace]`, `[workspace.dependencies]`, `default-members`); [SDD 03](03_mapa_de_codigo.md); testes no host (ELF e cpio inválidos, propriedade dos direitos, entrelaçamento de handles).

### Decisão 8 — pasta `user/` e a regra 5 do guidelines 03

**Casos 1** (pasta nova de primeiro nível; regra 8) **e 3** (a regra 5, "só o `kernel/` depende de adaptador", muda).

| Opção | Prós | Contras |
| --- | --- | --- |
| **A. `user/` (runtime e programas), com os adaptadores `arch/<isa>-user`** | O `asm!` continua só em `arch/`; o `unsafe` do `user/` é mínimo e com `// SAFETY:` (só o `user/rt`: `GlobalAlloc`, auxv, `#[unsafe(no_mangle)]`), e os programas ficam em `forbid(unsafe_code)` | Uma pasta nova e uma regra alterada |
| B. Programas em `kernel/src/bin` | Nada novo | Programa de usuário compila com a árvore do kernel; mistura as camadas |
| C. O stub dentro de `arch/<isa>` com `cfg` | Sem pasta de crate nova | O programa de usuário liga o crate do kernel inteiro (`global_asm!` de vetores) |

**Recomendação:** A. **O diff está feito em [`docs/guidelines/03_camadas_e_dependencias.md`](../guidelines/03_camadas_e_dependencias.md)** (regra 5 alterada, regras 11 e 12 novas): o merge do Draft o aprova. **O PR da 4a traz o `check-layers` conhecendo `user/` e a regra 12, com o teste vermelho antes do verde:** hoje `arch/x86_64-user` seria classificado como `Adapter` e o `Kernel` poderia depender dele; `user/` não classifica em camada nenhuma.

**O que muda:** `Cargo.toml` (`default-members` sem `user/` nem `arch/*-user`, que só compilam para os alvos bare-metal); `xtask/src/checks/layers.rs` (a camada `User`); [SDD 03](03_mapa_de_codigo.md) e [SDD 05](05_build_e_execucao.md).

### Decisão 9 — overcommit na ABI nativa

**Caso 6** (comportamento visível a programa Linux; [SDD 08](08_roadmap.md#decisões-futuras-casos-do-agentsmd)).

| Opção | Prós | Contras |
| --- | --- | --- |
| **A. Contabilidade estrita por padrão:** `vmo_create` e `vmar_map` reservam páginas contra o total; as páginas seguem preguiçosas | Falta de memória vira `NoMemory` numa chamada, tratável, não um processo morto na falta de página | Reserva mais que o uso real (custo **não medido**); um contador global |
| B. Heurístico (recusa só o absurdo) | Programa Linux (`malloc` de sobra) se comporta como espera | Falta de página sem frame mata o processo; não há matador de OOM |
| C. Irrestrito | Nada a contar | Um `vmo_create` gigante não falha; esgotar a RAM derruba o sistema |

**Limite que a recomendação não resolve:** um contador **global** estrito, sem limite
**por processo**, deixa um processo reservar a RAM inteira e negar `NoMemory` aos outros
até os grupos de recursos da fase 9. A fase 4 só impede que o total passe da RAM.

**Recomendação:** A para a ABI nativa, com a política **por processo**: o adaptador Linux
da fase 5 poderá pedir o modo heurístico para os programas dele. O modo exato do Linux
(`vm.overcommit_memory`) **NÃO VERIFICADO**.

**O que muda:** um contador de reserva no kernel; `NoMemory` em `vmo_create` e `vmar_map`; o campo de política no Process.

## Decisões do agente (para o dono vetar)

O `AGENTS.md` manda o agente decidir o que segue o desenho e as regras. Estas ficaram
fora da lista acima por não serem dos seis casos; cada uma diz o que a faria subir.

| Decisão | O que o agente decidiu | Contingência |
| --- | --- | --- |
| **KPTI** | Só em CPU vulnerável (detecção: `IA32_ARCH_CAPABILITIES.RDCL_NO`, bit 0, **verificado** em `msr-index.h`; mais uma lista de CPUs imunes, **NÃO VERIFICADO**), com chave de override `kpti=on\|off\|auto` na linha de comando do Limine (para medir as duas configurações). Sem informação, vulnerável. O [SDD 08](08_roadmap.md) e o [SDD 09](09_seguranca.md) já dizem "só em CPU vulnerável" | Se o dono quiser sempre ligado, a 4e perde a detecção e o override; o resto não muda |
| **Framebuffer e entrada** | Ficam na fase 5, onde o [SDD 08](08_roadmap.md#decisões-futuras-casos-do-agentsmd) já os lista. O desenho deixa o caminho (VMO físico com política de cache no `vmar_map`; teclado por canal) | A interface é caso 6 quando chegar; nada a decidir na fase 4 |
| **Alvo de compilação dos programas** | Os mesmos alvos `*-unknown-none`, com flags próprias (`code-model=small`, `relocation-model=static`), linker script próprio e `--target-dir` separado ([SDD 05](05_build_e_execucao.md): sem nightly nem `build-std`) | Se o teste da 4b mostrar que o `code-model` não se sobrepõe, a saída é alvo JSON, que exige `build-std`: **volta ao dono** (ferramenta de build nova, caso 2) |
| **`arch/<isa>-user`** | Crates de adaptador do lado do usuário (`asm!` de `syscall`/`svc`, de `_start`, de `xmm0`/`v0` do teste). Cabem nas regras 4 e 8 do guidelines 03 (adaptador, crate numa pasta existente) | — |
| **ABI no `hal`** | Módulo `abi` em `core/hal` (a "porta de entrada" do [SDD 02](02_arquitetura.md)): números, `Status`, direitos, mensagens, `USER_TOP`. Porta nova no `hal` é do agente | Um crate `core/abi` é a opção A' da decisão 7 |

## O diff dos guidelines

`docs/guidelines/03_camadas_e_dependencias.md` já traz as regras alteradas e novas
(regra 5; regras 11 e 12 para `user/`, `arch/<isa>-user` e o `unsafe` mínimo do `user/rt`), como o `AGENTS.md` pede do Draft. As outras decisões não
tocam o guidelines. O `make layers-check` só aprende a camada `User` no PR da 4a.

## Fontes lidas e o que ficou por ler

| Fonte | Usada para | Estado |
| --- | --- | --- |
| Intel SDM via [felixcloutier](https://www.felixcloutier.com/x86/sysret) (`SYSCALL`, `SYSRET`) | Registradores; `CS` = `STAR[63:48]`+16, `SS` = +8, RPL forçado a 3; `#GP` por `RCX` não canônico | Lida (secundária) |
| [Xen XSA-7](https://xenbits.xen.org/xsa/advisory-7.html) | CVE-2012-0217 | Lida |
| [Linux `entry_64.S`](https://github.com/torvalds/linux/blob/master/arch/x86/entry/entry_64.S), [`cpu/common.c`](https://github.com/torvalds/linux/blob/master/arch/x86/kernel/cpu/common.c), [`idt.c`](https://github.com/torvalds/linux/blob/master/arch/x86/kernel/idt.c) | Entrada, `FMASK`, `STAR`, `PCIDE`, Meltdown, `#DB` com IST, `fixup_bad_iret` | Lidos (cópia local da pesquisa) |
| [Linux `syscall_64.c`](https://github.com/torvalds/linux/blob/master/arch/x86/entry/syscall_64.c) | A condição de `SYSRET` contra `IRET` | Lida nesta rodada |
| [Linux `msr-index.h`](https://github.com/torvalds/linux/blob/master/arch/x86/include/asm/msr-index.h) | Números dos MSRs; `ARCH_CAP_RDCL_NO` = `BIT(0)` | Lida nesta rodada |
| [Linux `entry.S` (arm64)](https://github.com/torvalds/linux/blob/master/arch/arm64/kernel/entry.S), [`esr.h`](https://sources.debian.org/src/linux/6.12.69-1/arch/arm64/include/asm/esr.h), [`arch_timer.c`](https://github.com/torvalds/linux/blob/master/drivers/clocksource/arm_arch_timer.c) | Vetores, `sp_el0`, KPTI por `TTBR1`, `EC`, acesso de EL0 ao contador | Lidos |
| [syscall(2)](https://man7.org/linux/man-pages/man2/syscall.2.html), [`auxvec.h`](https://github.com/torvalds/linux/blob/master/include/uapi/linux/auxvec.h), [`elf-em.h`](https://github.com/torvalds/linux/blob/master/include/uapi/linux/elf-em.h), `elf.h` das duas ISAs | Convenção de registradores; `AT_*`, `e_machine` e `R_*_RELATIVE` | Lidos nesta rodada |
| [inode(7)](https://man7.org/linux/man-pages/man7/inode.7.html) | `S_IFMT` e os tipos de arquivo | Lida nesta rodada |
| [Linux PTI](https://docs.kernel.org/arch/x86/pti.html), [entry_64](https://docs.kernel.org/arch/x86/entry_64.html), [x86_64 mm](https://docs.kernel.org/arch/x86/x86_64/mm.html), [arm64 memory](https://docs.kernel.org/arch/arm64/memory.html) | Trampolim, `swapgs`, layout | Lidos |
| Zircon: [rights](https://fuchsia.dev/fuchsia-src/concepts/kernel/rights), [handle_replace](https://fuchsia.dev/fuchsia-src/reference/syscalls/handle_replace), [channel_write](https://fuchsia.dev/fuchsia-src/reference/syscalls/channel_write), [process_create](https://fuchsia.dev/fuchsia-src/reference/syscalls/process_create), [channel_create](https://fuchsia.dev/fuchsia-src/reference/syscalls/channel_create) | Direitos, atomicidade do handle, limites do canal | Lidas |
| [API do seL4](https://docs.sel4.systems/projects/sel4/api-doc.html), [Redox, schemes](https://doc.redox-os.org/book/ch05-06-scheme-operation.html) | Contraste | Lidas (direitos do seL4 não; o namespace do Redox está em mudança) |
| [cpio `newc`](https://docs.kernel.org/driver-api/early-userspace/buffer-format.html) | Gramática, alinhamento, `c_mode`, `c_namesize`, hard link, trailer | Lida (reconferida nesta rodada) |
| `PROTOCOL.md` v8.x do Limine (cópia local) | Pedido de módulo, `limine_file` | Lida; **a versão em uso (12.10.0) não foi conferida** |
| [mmap(2)](https://man7.org/linux/man-pages/man2/mmap.2.html), [signal(7)](https://man7.org/linux/man-pages/man7/signal.7.html), [getauxval(3)](https://man7.org/linux/man-pages/man3/getauxval.3.html), [binfmt_elf.c](https://github.com/torvalds/linux/blob/master/fs/binfmt_elf.c), [Hansen, lkml](https://lkml.iu.edu/1712.0/03977.html) | Guarda de pilha, `MAP_FIXED_NOREPLACE`, sinais, auxv, validação de ELF, custo de TLB | Lidos |

**Por ler antes de codar** (cada item é pré-requisito da fatia que o usa):

| Item | Fatia |
| --- | --- |
| Intel SDM: `IRETQ` com `NT = 1` (inferência: `#GP`) e com `RSP` não canônico (inferência: `#SS`/`#GP`), e os bits de `RFLAGS` que o `IRETQ` recusa | 4a |
| Arm ARM: `TCR_EL1` (`A1`, `T0SZ`, `TG0`, `EPD0`), `ISS` do SVC, `AP[2:1]`, `nG`, ASID, PAN com `LDTR`/`STTR`, `UAO`, `WXN` sobre EL0, bit de `CNTKCTL_EL1.EL0VCTEN`, `ERET` ilegal | 4a, 4e |
| Intel SDM: PCID (`CR3` bit 63, `INVPCID`), `IRET` com `RSP` não canônico, `CPUID` em anel 3 | 4a, 4e |
| Linux `mm/memory.c` (`do_anonymous_page`, `wp_page_copy`) | 4c |
| psABI x86-64 e aarch64 (pilha inicial, alinhamento); gABI (`EI_*`, congruência de `p_vaddr`/`p_offset`) | 4b |
| Limine `CONFIG.md` (chave do módulo) e a revisão do `PROTOCOL.md` em uso | 4b |
| Doc do Cargo: precedência de `CARGO_ENCODED_RUSTFLAGS` contra `target.<triple>.rustflags`; teste real do `-C code-model=small` | 4b |
| Zircon: `zx_process_start`; *peer closed* e o descarte de handles na morte da ponta | 4c |
| Linux: `spectre.html`, `mds.html` e a técnica de limitar índice de *array* (Spectre v1) | 4e |

Levantamento de novembro de 2026.
