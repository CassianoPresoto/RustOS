# SDD 06 — Testes

> **O que este capítulo responde:** quais andares de teste existem, onde cada um
> roda, o comando, e o que ainda não existe.

## Os andares

| Andar | O que prova | Onde roda | Comando |
| --- | --- | --- | --- |
| 1. Unidade da política | Algoritmo de `core/` certo, inclusive bordas (frame parcial, bitmap no limite, fila cheia) | host | `make test` |
| 2. Adaptador com porta falsa | Driver e caminhada de tabela fazem a sequência certa de acessos, sem hardware | host | `make test` |
| 3. Fumaça na QEMU, modo de teste | O kernel sobe de verdade, roda todos os autotestes e medidas e chega ao `boot ok` nas duas ISAs; só na imagem com a feature `selftest` | QEMU | `make smoke ARCH=all` |
| 3c. Fumaça na QEMU, modo normal | O boot que o usuário vê chega ao `boot ok` e imprime o tempo; barato (1,7 s no x86_64, 0,2 s no aarch64 em HVF com o firmware). Na imagem enxuta (`release-small`, sem a feature `selftest`) confere também a recusa do modo de teste e do `bench` | QEMU | `make smoke-fast ARCH=all [PROFILE=release-small]` |
| 3b. Tela na QEMU | O que o kernel desenha: fonte, terminal, shell e barra, depois de digitar comandos | QEMU | `make screenshot ARCH=all CMDS="fetch"` |
| 4. Gates de arquitetura | Camadas, `unsafe`, `asm!`, comentários, docs | host | `make check` |

## Andar 1 — política

Todo crate de `core/` e `lib/` é `#![cfg_attr(not(test), no_std)]`: no teste, tem
`std`; no kernel, não. Exemplos: `core/mm/src/frame.rs` (alocação, dupla liberação,
faixas recortadas, palavras parciais), `core/mm/src/buddy.rs` e `heap.rs` (400 mil e
300 mil passos aleatórios contra um modelo `BTreeMap`: nada se sobrepõe, o conteúdo
sobrevive e tudo volta ao estado de partida), `lib/sync/src/spin.rs` (4 threads × 10 000
incrementos — foi esse teste que achou o `then_some` que soltava o lock alheio).

`core/mm/src/vmar.rs`: 30 000 operações aleatórias (`reserve`, `allocate` nas duas
colocações, `unmap`, `find`) em cinco configurações de capacidade e tamanho, com semente
fixa, contra um modelo ingênuo (lista ordenada, candidatos achados página a página); a cada
passo as duas árvores são iguais, sem sobreposição e sem W+X. Mais as bordas: faixa no topo
do espaço (`base + len` estoura), alinhamento de `2^63`, espaço cheio, fragmentação, partir
sem vaga, uniformidade do sorteio (16 000 sorteios em 8 posições), o viés da rejeição e
`aslr_base` com intervalo menor que uma página.
`core/mm/src/asid.rs`: 100 000 trocas aleatórias em 4 CPUs e 40 espaços, com uma
**simulação de TLB por CPU** que falha se uma CPU carrega um índice com entrada de outro
espaço, e que o índice carregado numa CPU que ainda roda o espaço seja o do valor atual dele
(a CPU 0 quase não troca); o mesmo roteiro sem esvaziar tem de falhar. O roteiro
`double_rollover_idle_cpu_keeps_index` fixa o caso de dois esgotamentos com uma CPU ociosa.

Medição de `core/mm` no host, fora do `make test` (testes `#[ignore]`):

```sh
cargo test --release -p rustos-mm bench -- --ignored --nocapture --test-threads=1
```

## Andar 2 — adaptador com porta falsa

| Teste | Porta falsa |
| --- | --- |
| `drivers/uart16550`, `drivers/pl011` | `FakeBus: RegisterBus` que grava cada escrita e simula FIFO cheia |
| `core/mm/src/tables.rs` | `Toy: PageFormat` (mesma forma dos formatos reais), `FakeTables: TableMemory` em `HashMap`, `Cpu: MmuControl` que conta as publicações e `Frames: FrameSource` em `Vec`; mapeia página de 4 KiB e de 2 MiB, recusa W+X e percorre as folhas |
| `arch/*/src/pte.rs`, `descriptor.rs` | A codificação: ida e volta de cada `PageFlags`, e o `PageFormat` que distingue tabela, folha grande e entrada vazia |
| `core/mm/src/ranges.rs` | Só aritmética: cabeça e cauda desalinhadas, virtual e físico que discordam no alinhamento, região menor que 2 MiB, regiões adjacentes, sobrepostas e vazias |
| `core/mm/src/wx.rs` | Lista de `Mapping` montada à mão: W+X, seção com permissão trocada, alias da imagem no mapa direto, metade baixa |
| `boot/limine` | Layout das structs contra `limine.h` (`size_of`, `offset_of`), inclusive o pedido e a resposta de MP nas duas ISAs, tradução dos tipos de região e do formato do framebuffer |
| `arch/x86_64/src/lapic.rs`, `arch/aarch64/src/sgi.rs` | O ICR (destino, vetor, atalho "todos menos eu") e o valor de `ICC_SGI1R_EL1` (afinidade, INTID, `IRM`, faixa `RS`), com `ApicRegisters` falso |
| `drivers/gicv3` | Os frames do GICR percorridos até `Last` (frames de GICv3 e de GICv4), a SGI habilitada antes do `ISENABLER` |
| `core/sched/src/migration.rs` | Duas filas, 200 sementes: a tarefa está em exatamente uma, o lag viaja, a corrente recusa `detach` ([SDD 14](14_escalonador.md)) |
| `core/mm/src/heap.rs` | `FakeMemory: FrameMemory`, uma faixa de palavras que derruba o teste em acesso fora dela, desalinhado ou abaixo da base |
| `core/cpio` | `newc` montado no teste (todas as combinações de nome e dado de 0 a 9 bytes, NUL a mais no nome) e o de um `cpio -o -H newc` real colado como constante; uma recusa por regra, truncação em todo comprimento, os 13 campos com dígito inválido e os de tamanho em 0xffffffff e vizinhos |
| `core/elf` | ELF montado no teste, com cada campo escrito como está; uma recusa por linha da tabela de validação, estouro em `p_offset`, `p_vaddr`, `p_filesz`, `p_memsz`, `e_phoff` e `e_entry`, truncação em todo comprimento, corrupção de cada byte do cabeçalho; e dois ELF reais (x86_64 e aarch64) só com cabeçalho e program headers |
| `core/shell` | `Host` falso e um terminal VT mínimo (`testing.rs`) que confere prompt, linha e cursor depois de cada tecla, nas duas regras de última coluna |
| `core/term` | Fuzz determinístico (xorshift, 300 mil bytes por tamanho de grade) que confere cursor, região e anel; o mesmo fluxo em bloco e byte a byte dá a mesma grade. Um segundo fuzz, com tela alternativa e histórico, mistura trocas de tela, `ED 3`, conjuntos G0/G1, tabulação e rolagem da visão a bytes soltos e a CSI bem formado, e confere também o histórico e a visão |

O que tem `asm!` só compila com `target_os = "none"`; a parte pura do mesmo crate
(`descriptor.rs`, `pte.rs`) roda no host. A caminhada de tabela é uma só, em
`core/mm`; o `mmu.rs` de cada ISA tem só o que precisa de `asm!` e de ponteiro.

## Andar 3 — fumaça

`cargo xtask smoke` monta a ESP com a linha de comando `rustos.selftest=1` (o [modo de teste](04_boot.md#modos-de-boot)), sobe a QEMU sem tela e lê a serial. Passa ao ver
`[rustos] boot ok`; falha com pânico (`[rustos] panic`), com a QEMU saindo antes,
ou em 120 s. O marcador é a constante `BOOT_OK_MARKER` do kernel e `BOOT_OK` do
xtask: os dois mudam juntos. O kernel só imprime o marcador se os autotestes de
frame, de heap (`heap self-test`), de W^X (`w^x: ok`), de interrupção e de tarefas
(`task self-test`: preempção e estado de SIMD por tarefa, [SDD 15](15_tarefas_e_preempcao.md))
passarem; o `smoke` exige a linha `task self-test: ok`, a `smp self-test: ok` (migração entre núcleos, [SDD 15](15_tarefas_e_preempcao.md)) e `cpus: N online`, com o N que a QEMU recebeu (`-smp`). Exige `kaslr: on` e reprova endereço do kernel (`0xffff…`) em linha `[rustos]` ([SDD 09](09_seguranca.md#endereços-do-kernel-no-log)). O de
W^X percorre as tabelas novas ([SDD 13](13_memoria_virtual.md)); `fault text` no shell
confere que a CPU recusa a escrita em `.text`.

Como o modo de teste imprime o que o normal não imprime, o `smoke` exige também a linha de cada autoteste e medida (`frame`, `heap` e `exception self-test: ok`, `timer:`, `idle:`, `key residue:`, `zero bench:`, `random bench:`, `switch:`, `scale:`, `alloc storm:`, `stacks:`) e a `boot time:`: se a linha de comando se perder, o boot normal não passa por boot de teste.

A linha `stacks:` (pico de pilha por tipo de thread e uma prova da marca d'água) é gate: vira `FAILED` se um pico passa de 75% da pilha ou se a prova falha ([SDD 15](15_tarefas_e_preempcao.md#pico-de-pilha)). Duas linhas são medida e heurística, não gate: `key residue:` (varredura da pilha
atrás de cópias da chave, com uma sonda que planta 4 estados e exige achá-los; [SDD 09](09_seguranca.md))
e `zero bench:` ([SDD 07](07_desempenho_e_recursos.md)). O kernel as imprime e segue;
o `smoke` reprova o `FAILED` de qualquer linha. O autoteste do heap confere que o zerar
rápido mexe só na faixa pedida ([SDD 13](13_memoria_virtual.md), "Zerar").

## Andar 3c — modo normal (`make smoke-fast`)

`cargo xtask smoke-fast` sobe a mesma QEMU sem linha de comando. Exige o `boot ok`, a `boot time: kernel N ms to prompt`, as linhas que os dois modos têm (`protections:`, `w^x: ok`, `reclaimed:`, `kaslr: on`, `cpus: N online`, `frame self-test: ok`, `exception self-test: ok`) e que nenhuma linha do modo de teste apareça (a lista é a do modo de teste menos a do normal, mais `task`, `smp` e `wake self-test:`): um modo normal que volta a rodar o que o de teste roda é regressão de tempo. Imprime o tempo do kernel até o prompt (X ms) e o de parede, e reprova se o kernel passar de um teto folgado (50 ms com `RUSTOS_ACCEL=hvf` ou `kvm` forçado, 500 ms no resto: a QEMU pode cair no TCG sem avisar, como no runner do CI; o boot normal leva 2 a 40 ms). Entra no `make pr` e no CI.

O que o modo normal deixa de provar (heap, timer, preempção, SIMD por tarefa, migração) só o `make smoke` prova.

**A imagem enxuta (`PROFILE=release-small`, sem a feature `selftest`, [SDD 05](05_build_e_execucao.md#perfis)).** O `make smoke` não a cobre: ela não tem o modo de teste. O `smoke-fast` dela faz dois boots. O primeiro é o normal acima, com os mesmos tetos. O segundo passa `rustos.selftest=1` (o `limine.conf` é o mesmo do `make smoke`) e exige `[rustos] selftest: not in this build`, as linhas do modo normal, nenhuma do modo de teste e, depois do `boot ok`, que o `bench` digitado no shell responda `bench: not in this build`. Sem isso a imagem de produção, que o CI não roda no modo de teste, apodreceria sem ninguém ver: um `cfg` errado que tire o `w^x:` ou o `reclaimed:` só apareceria nela. O `smoke-fast` confere também a pilha: digita `ps` e reprova se o pico de alguma tarefa passa de 75% da pilha da própria linha, porque a linha `stacks:` e a sonda só existem com a feature. O `smoke-panic` da `release-small` (nove cenários, cerca de 21 s no x86_64 e 8 s no aarch64) roda também. Tudo entra no `make pr` e no CI nas duas ISAs. O `core/shell` tem o mesmo par: `cargo test` com o `bench` e `cargo test --no-default-features` sem ele (`make test`).

## Andar 3a — pânico (`make smoke-panic`)

`cargo xtask smoke-panic` derruba o kernel de propósito em oito pontos, mais um boot que roda `mem`, `ps` e `fetch`, e confere que a mensagem
aparece na serial e que nenhum endereço do kernel vem junto (o log inteiro depois do primeiro `[rustos]`, com o corpo do pânico e a saída do shell). Constrói com a feature `panic-tests`
(além das de `RUSTOS_FEATURES` e da `selftest` do perfil) e sobe um boot por cenário:

| Cenário | Como provoca | Mensagem esperada |
| --- | --- | --- |
| Pânico na entrada, antes da memória | linha de comando `rustos.panic_early=1` | `[rustos] panic:` e `panic_early: Entry` |
| Falha na entrada, antes dos vetores do kernel | `rustos.panic_early=2` (leitura de um endereço sem mapa) | `[rustos] fault: page fault at 0x1000` e `at pc kernel+0x…` |
| Pânico com as tabelas novas ativas, antes do console | `rustos.panic_early=3` | `[rustos] panic:` e `panic_early: Activated` |
| Falha com a serial e a tela presas | `fault lock` no shell | `[rustos] fault: page fault at …` |
| Pânico dentro da mensagem do pânico | `fault nested` | a mensagem do segundo pânico |
| Pânico em todas as CPUs ao mesmo tempo | `fault all` (uma tarefa por CPU ativa, com barreira) | um único `[rustos] panic:`, sem `failed again` nem texto misturado |
| Ceder a CPU com um `IrqLock` preso | `fault held` (o shell segura um lock e chama `yield_now`; vale em `release` também, [SDD 15](15_tarefas_e_preempcao.md#regra-de-lock)) | `context switch with 2 IrqLock held, expected 1` |
| Escrita em `.text` | `fault text` | `page fault at kernel+0x…` (deslocamento, não endereço) |

O boot que roda `mem`, `ps` e `fetch` reprova também se o `ps` mostra o pico de pilha do shell acima de 48 KiB (75% de 64), na coluna `stack` ([SDD 15](15_tarefas_e_preempcao.md#pico-de-pilha)); vale o perfil do `make smoke-panic PROFILE=…`.

A linha de comando vem do `cmdline:` da entrada do `limine.conf` (`xtask/src/image.rs`); `core/hal`
`BootParams::parse` a traduz e tem teste de entrada inválida. Os cenários de shell (`fault lock`, `fault all`…) sobem no modo normal. Custa nove boots por ISA (cerca de
27 s no x86_64 e 54 s no aarch64 neste host); `fault lock`, `fault nested`, `fault all` e `fault held` só existem no build com `panic-tests`, por isso é alvo à parte; entra no `make pr` e no CI,
não no `make smoke`. Com `debug-addrs` ele recusa rodar: os cenários conferem o endereço relativo.

## Andar 3b — tela

`cargo xtask screenshot` sobe a QEMU sem janela, espera o `boot ok`, digita cada
comando na serial, pede `screendump` pelo monitor (socket Unix) e converte o PPM
em PNG com o codificador do próprio xtask (`xtask/src/png.rs`). No CI, a tela de
cada ISA fica como artefato do run. Ninguém compara pixel ainda: a imagem é para
o review ver o que o kernel desenhou.

## O que ainda não existe

- **Teste dentro do kernel** (`#[ktest]` no estilo do Asterinas, com saída por
  `isa-debug-exit` no x86 e PSCI `SYSTEM_OFF` no aarch64): entra quando houver
  código de kernel que só se testa rodando — interrupções, fase 1.
- **Cobertura**: sem ferramenta, porque `cargo-llvm-cov` é crate externo
  (decisão do caso 2 do `AGENTS.md`).
- **Hardware real**: nenhum boot fora da QEMU ainda.
