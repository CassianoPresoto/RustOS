# SDD 16 — O terminal

> **O que este capítulo responde:** o que o emulador de `core/term` entende, de quem é
> cada memória (grade, tela alternativa, histórico), como o teclado e o kernel o
> usam, e contra o quê cada comportamento foi conferido.

## A memória é do kernel

O `Terminal` não aloca. O kernel entrega três fatias de `Cell` (8 bytes cada), e
cada uma é opcional depois da primeira:

| Fatia | Como entra | Tamanho | Sem ela |
| --- | --- | --- | --- |
| Grade principal | `Terminal::new(cells, cols, rows)` | `cells_needed(cols, rows)` | o terminal não existe |
| Grade alternativa | `set_alternate(alt)` | o mesmo da principal | `?47`, `?1047` e `?1049` não trocam de tela; `?1049h` também não limpa a principal |
| Histórico | `set_history(cells)` | `history_cells_needed(cols, linhas)` | `Shift+PageUp` não rola nada |

`set_alternate` e `set_history` valem a qualquer momento, porque o estado do
terminal é sempre válido: `set_history` começa vazio e leva a visão à tela viva;
`set_alternate` recusa (`false`) fatia curta e recusa com a alternativa ativa,
quando a principal é que está guardada na fatia antiga. Fatia maior que a
necessária é usada só até o tamanho da grade.

`new` recusa `rows > MAX_ROWS` (256) e `cols > MAX_COLS` (512): são os tamanhos dos
bitmaps de linhas sujas e de paradas de tabulação. O kernel
(`kernel/src/screen.rs`) aloca as três com `Memory::alloc_slice` e corta `cols` e
`rows` nesses limites.

O histórico só é alocado com teclado (`platform::has_keyboard()`), e depois de
`interrupts::init`, que é quando o x86_64 sabe se o PS/2 respondeu; as linhas que
rolaram antes disso não ficam. No aarch64 da `virt` não há teclado, ninguém rolaria
a visão, e os 252 KiB ficariam parados: o boot diz `history 0 lines`
([DEBT-0050](../tech-debt/0050-historico-sem-rolagem-no-aarch64.md)). O boot imprime
`screen: LxA, grid CxR, alt on|off, history N lines`, então falta de memória não
passa em silêncio. O custo em bytes está no [SDD 07](07_desempenho_e_recursos.md).

## O que o emulador entende

| Sequência | Efeito |
| --- | --- |
| `?1049h` / `?1049l` | Salva o cursor (DECSC), vai para a grade alternativa e a limpa; ao sair, volta à principal e restaura o cursor (DECRC) |
| `?1047h` / `?1047l` | Vai para a alternativa sem limpar; ao sair, limpa a alternativa antes, se estiver nela |
| `?47h` / `?47l` | Troca de grade sem limpar e sem mexer no cursor |
| `?1048h` / `?1048l` | Só DECSC e DECRC |
| `ED 3` (`CSI 3 J`) | Apaga o histórico; não toca a tela, o cursor nem a quebra pendente |
| `ESC ( 0`, `ESC ) 0` | G0 e G1 viram o DEC Special Graphics |
| `ESC ( B`, `ESC ) B` | G0 e G1 voltam a ASCII; qualquer outro final também vira ASCII (sem NRCS) |
| `SO` (0x0E), `SI` (0x0F) | GL passa a G1 e volta a G0 |
| `ESC H` (HTS) | Parada de tabulação na coluna do cursor |
| `CSI g`, `CSI 0 g` (TBC) | Apaga a parada da coluna do cursor |
| `CSI 3 g` (TBC) | Apaga todas; outros parâmetros são ignorados |
| `CSI Ps I` (CHT), `CSI Ps Z` (CBT) | Anda `Ps` paradas para a frente ou para trás |
| `HT` (0x09) | Próxima parada; sem parada à frente, a última coluna |

Paradas começam a cada 8 colunas e voltam a isso no `ESC c`. O resto do conjunto
(cursor, ED, EL, ICH, DCH, IL, DL, SU, SD, DECSTBM, DECOM, DECAWM, SGR, DSR, DA) é o
da fase 0.5 e está nos testes de `core/term/src/terminal/tests.rs`.

As duas grades dividem região de rolagem, modos, cursor, caneta, conjuntos de
caracteres e paradas. O DECSC tem um espaço por grade.

### A tela alternativa

Cada grade é uma fatia com o próprio anel de linhas; a troca só permuta as duas
fatias, então não copia célula. Na alternativa:

- não há histórico: rolar nela não grava nada e `scroll_view_back` não faz nada;
- `ESC c` limpa a alternativa, volta à principal e limpa a principal.

### O histórico de rolagem

Anel de linhas inteiras de capacidade fixa. Uma linha entra nele quando o `scroll_up`
tira a linha 0 da região, com a região começando no topo e a principal ativa: `LF`
no fim da tela, `IND`, `NEL`, `SU` e `DL` na linha 0. `ED 2`, `IL` e regiões que não
começam no topo não gravam. Cheio, o anel descarta a mais antiga.

A visão é um deslocamento (`view_offset`, 0 é a tela viva). `Terminal::row` já devolve
a linha da visão, do histórico ou da grade, então o `render` do kernel não muda.
Com a visão rolada o cursor fica invisível, e qualquer `write` não vazio volta à tela
viva (o `scrollTtyOutput` do xterm, padrão `True` em `charproc.c`). `scroll_view_back`, `scroll_view_forward` e
`snap_to_live` marcam a tela inteira suja, só se a visão mudou.

### O desenho de caixas

O `Terminal` guarda o caractere Unicode: com G0 em DEC Special Graphics, `q` vira `─`
na célula. Quem desenha decide o glifo; sem glifo, `gfx::font::glyph` devolve o
substituto (U+FFFD). Só os caracteres ASCII 0x5F a 0x7E mudam; `_` vira espaço.

| DEC | Unicode | Fonte tem? | DEC | Unicode | Fonte tem? |
| --- | --- | --- | --- | --- | --- |
| `` ` `` | ◆ U+25C6 | sim | `t` | ├ U+251C | sim |
| `a` | ▒ U+2592 | sim | `u` | ┤ U+2524 | sim |
| `b` `c` `d` `e` `h` `i` | ␉ ␌ ␍ ␊ ␤ ␋ (U+2409 a U+2424) | não | `v` | ┴ U+2534 | sim |
| `f` | ° U+00B0 | sim | `w` | ┬ U+252C | sim |
| `g` | ± U+00B1 | sim | `x` | │ U+2502 | sim |
| `j` | ┘ U+2518 | sim | `y` `z` | ≤ ≥ U+2264 U+2265 | sim |
| `k` | ┐ U+2510 | sim | `{` `\|` | π ≠ U+03C0 U+2260 | sim |
| `l` | ┌ U+250C | sim | `}` | £ U+00A3 | sim |
| `m` | └ U+2514 | sim | `~` | · U+00B7 | sim |
| `n` | ┼ U+253C | sim | `o` `p` `r` `s` | ⎺ ⎻ ⎼ ⎽ U+23BA a U+23BD | sim (linhas 0, 4, 12 e 15) |
| `q` | ─ U+2500 | sim | | | |

Só os seis símbolos de controle (␉ ␌ ␍ ␊ ␤ ␋) caem no substituto
([DEBT-0048](../tech-debt/0048-dec-sem-simbolos-de-controle-nem-nrcs.md)).

## O teclado e o kernel

`Keyboard` devolve `Key::ScrollBack` para Shift+PageUp e `Key::ScrollForward` para
Shift+PageDown. A decisão está em `Terminal::press`, testada no host: as duas
rolam a visão em meia tela (`rows / 2`, no mínimo 1) e não geram byte, então o
programa não as vê; qualquer outra tecla volta à tela viva antes de virar bytes
(`encode_key`). `Screen::key_event` só chama `press` e deixa a visão suja.

Quem desenha é o `console::flush` do laço do shell, fora da IRQ: a IRQ do teclado
acorda a tarefa (`notify_input`), e `read_input_blocking` chama `flush` antes de
cada sono, já com a IRQ mascarada, mesmo sem byte. Fora da máscara, uma tecla de
rolagem que chegasse entre o `flush` e o sono não acordaria ninguém (o
`INPUT_WAITER` já foi consumido) e a visão ficaria sem desenho. O `flush` segura o lock da tela com IRQ mascarada, como em toda
saída; o pior caso medido está no [SDD 07](07_desempenho_e_recursos.md)
([DEBT-0051](../tech-debt/0051-redesenho-da-tela-com-irq-mascarada.md)).

O shell tem dois comandos de demonstração: `lines` desenha uma tabela com o DEC
Special Graphics e `alt` / `alt off` entra e sai da tela alternativa. Sem grade
alternativa (`Host::has_alternate_screen` falso), `alt` avisa e não envia `?1049`.

## Comandos que falam com processos (M1)

Três comandos do shell tratam de programas do initramfs; a porta é o `Host` do `core/shell`
(`programs`, `run_program`, `TaskInfo::pid`), e o kernel a implementa em `kernel/src/programs.rs`. Os
textos são em inglês, como a saída do sistema, e o `core/shell` segue puro e testado no host com um `Host` falso.

| Comando | O que faz | Saída |
| --- | --- | --- |
| `ls` | Lista os arquivos do initramfs | `bytes  name` e uma linha por arquivo, em ordem; `ls: no initramfs in this boot` sem initramfs |
| `run <programa> [&]` | Carrega o ELF, roda como processo isolado com o canal de boot e o console, **espera** e mostra como acabou. Com `&` (solto ou colado ao nome) não espera | `exit code N (pid P)`; `killed: <causa> (pid P)` (`page fault on a write to 0x…`, `… (null or low address)`, `… a kernel address` sem endereço, `illegal instruction`, `out of memory`…); `started pid P` com `&`; `run: X: no such program (try 'ls')`; `run: X: too many processes` com a tabela de tarefas cheia. Cada resposta sai numa escrita só |
| `ps` | Ganha a coluna `pid` (depois de `name`): o processo que a tarefa roda, ou `-` na tarefa de kernel. A coluna `name` tem 16 caracteres e, na tarefa de um processo, mostra o programa (`count`, `hello`), cortado em 16 bytes sem partir um caractere | Uma linha por processo vivo, com o nome do programa |
| `kill <pid>` | Mata o processo vivo `pid` que o `run` lançou. Não espera a morte | `killed pid P`; `kill: no process with pid P`; `kill: pid P is already ending`; `usage: kill <pid>` sem pid decimal |

Com Ctrl-C na espera do `run` em primeiro plano, a resposta é `interrupted (pid P)`, e o processo morre ([SDD 17](17_processos_isolamento_e_syscalls.md#ctrl-c-e-kill)).

A saída do programa (`[user] …`) aparece na tela e na serial, antes do `exit code`. As linhas do kernel sobre o ciclo de vida (`[rustos] user: pid N exited …`) vão só para a serial: a tela mostra só o que o shell já diz. Argumentos do programa ainda não passam ([DEBT-0130](../tech-debt/0130-run-sem-argumentos-para-o-programa.md)).
O caminho do kernel está no [SDD 17](17_processos_isolamento_e_syscalls.md#o-m1-programas-pelo-shell-como-entrou).

## Saída de segundo plano

Um processo lançado com `&` segue escrevendo enquanto o shell mostra o prompt ou imprime a resposta de um comando. A regra é que **a saída de usuário nunca entra no meio do que o shell escreve** e **nunca fica colada no prompt**. Duas peças, uma pura e uma do kernel:

| Peça | Onde mora | O que faz |
| --- | --- | --- |
| `Backlog<N>` | `core/shell/src/backlog.rs`, testado no host | Fila limitada de texto: `push` entra **inteiro ou não entra**; o que não coube é contado; `drain` escreve o texto e, se algo se perdeu, a linha `[rustos] user output: N writes lost (backlog full)` |
| `Shell::interject` | `core/shell/src/shell.rs`, testado no host com o `Host` falso e a tela VT dos testes | Com a fila não vazia: sobe até a linha do prompt (`at / cols` linhas), apaga daí até o fim da tela (`\r\x1b[J`), deixa a fila sair a partir da coluna 0 e chama `redraw`, que reescreve prompt e linha em edição e põe o cursor onde estava |
| `userout` | `kernel/src/userout.rs` | A porta de saída do usuário: o `debug_write` (e as linhas `kprintln_serial!` do ciclo de vida) passam por ela, e ela escolhe entre a tela e a fila |

**Os dois estados.** Fora do `run` em primeiro plano (o estado normal, a partir do laço do shell), a saída de usuário vai **sempre para a fila** e acorda o laço do shell (o mesmo `notify_input` da entrada). O laço, depois de cada tecla e quando acorda sem tecla, chama `interject` se há o que mostrar. Assim a saída que chega com o shell no meio de um `ps`, de um `bench` ou do eco de uma tecla espera o comando acabar e entra **inteira**, depois do prompt redesenhado, e a que chega com o shell parado no prompt apaga o prompt, passa e o devolve. Durante o `run` em primeiro plano o shell dorme sem escrever, com o cursor na coluna 0: `programs::run` chama `userout::direct()`, que despeja o que esperava e deixa a saída ir direto à tela (é o que mantém `[user] hello…` antes do `exit code`), e `userout::queued()` ao voltar. Antes de o laço do shell existir (o `init` do modo de teste), o estado é direto.

**Tamanho e estouro.** A fila tem **1 024 bytes** (`BACKLOG_BYTES`, em `.bss`): cerca de doze linhas de 80 colunas. Uma escrita de usuário (`debug_write`, até 512 bytes mais o prefixo `[user] ` de cada linha) cabe inteira ou é descartada inteira; cheia, a fila conta as escritas descartadas e o shell, ao esvaziá-la, diz quantas (`[rustos] user output: N writes lost (backlog full)`). Uma linha nunca sai pela metade: toda escrita termina em `\n` (`debug_sanitize`, regra da 4a) e entra ou sai por inteiro. O lock da fila é um `IrqLock` e o despejo roda sob ele, em ordem `userout` → serial → tela, a mesma de todo `print`.

**As linhas só da serial.** O ciclo de vida (`entered ring3/el0`, `pid N exited`, `fault contained`) é diagnóstico da serial e a tela não o mostra. Com o prompt na tela elas também esperam na fila, marcadas com `U+0001` no começo; o despejo escreve o texto marcado só na serial, até o fim da linha. Na tela o prompt sai e volta no mesmo lugar, sem mudança visível. Sem isso a linha do kernel desalinhava a contagem de linhas do shell (o `redraw` seguinte apagava a linha errada na serial).

**Por que `\x1b[J` e não só `\x1b[K`.** A linha de comando longa quebra em várias linhas da tela; `\x1b[K` apagaria só a primeira. O shell sabe em que linha do prompt o cursor está (`at / cols`), sobe até ela e apaga até o fim da tela.

**O que fica de fora** (cada um é a mesma limitação da saída do shell hoje): com a tela alternativa (`alt`) ligada, a saída de fundo entra na alternativa ([DEBT-0151](../tech-debt/0151-saida-de-segundo-plano-entra-na-tela-alternativa.md)); o despejo roda com IRQ mascarada pelo tempo de escrever a fila (no máximo 1 KiB, a mesma ordem de um `print` de 512 bytes, [DEBT-0051](../tech-debt/0051-redesenho-da-tela-com-irq-mascarada.md)); e a fila cheia descarta ([DEBT-0150](../tech-debt/0150-fila-de-saida-de-segundo-plano-descarta-ao-encher.md)).

## Conferido contra o xterm

O `ctlseqs` do xterm (patch 411, 2026/08/23) não detalha tudo; o resto veio do
fonte do xterm no mesmo patch. O site `invisible-island.net` recusa clientes
automáticos (403): os arquivos foram lidos no espelho
`github.com/ThomasDickey/xterm-snapshots` (`ctlseqs.txt`, `charproc.c`, `cursor.c`,
`util.c`, `tabs.c`, `charsets.c`, `fontutils.c`).

| Comportamento | Fonte |
| --- | --- |
| `?47`, `?1047`, `?1048`, `?1049`: o que cada um liga e limpa | `ctlseqs.txt` (entradas `Ps = 4 7`, `1 0 4 7` a `1 0 4 9`) e `charproc.c` (`srm_ALTBUF`, `srm_OPT_ALTBUF`, `srm_OPT_ALTBUF_CURSOR`) |
| `1049h` limpa mesmo já estando na alternativa; `1049l` restaura o cursor mesmo na principal | `charproc.c`: `CursorSave; ToAlternate; ClearScreen` e `FromAlternate; CursorRestore`, com `ToAlternate`/`FromAlternate` só agindo se a grade for a outra |
| A alternativa não tem histórico | `ctlseqs.txt`, seção "The Alternate Screen Buffer" |
| Linhas só vão para o histórico na principal e com a região no topo | `util.c`, `xtermScroll` (`scroll_all_lines`) |
| `ED 3` apaga as linhas salvas e só isso | `ctlseqs.txt` (`CSI Ps J`) e `util.c`, `do_erase_display` |
| DECSC guarda posição, caneta, conjuntos, flag de quebra pendente e DECOM, um espaço por grade; DECRC em DECOM mantém o cursor na região | `cursor.c` (`CursorSave2`, `CursorRestoreFlags`, `sc[whichBuf]`) |
| `RIS` sai da alternativa, apaga histórico, paradas e conjuntos | `charproc.c`, `ReallyReset` |
| HTS, TBC 0 e 3, CHT, CBT; tabulação não cancela a quebra pendente; a busca ignora a parada sob o cursor | `ctlseqs.txt` e `tabs.c`, `charproc.c` (`CASE_HTS`, `CASE_TBC`, `CASE_CHT`, `CASE_CBT`) |
| DEC Special Graphics: 0x5F a 0x7E, a tabela Unicode | `charsets.c` (`nrc_DEC_Spec_Graphic`) e `fontutils.c` (`dec2ucs`) |
| SO e SI trocam GL entre G1 e G0 | `ctlseqs.txt` (SI, SO) |
| DECSTBM: `bot > top`, `bot` 0, omitido ou além da tela vale a última linha, cursor em casa | `charproc.c` (`CASE_DECSTBM`); já estava igual |
| DECOM: ligar e desligar levam o cursor para casa, dentro da região | `charproc.c` (`srm_DECOM`), `cursor.c` (`CursorSet`); já estava igual |
| Quebra pendente: o que a cancela (`CursorSet`, CUF/CUB/CUU/CUD, CR, IL, DL, ICH, DCH, EL, ED 0 a 2, ECH, LF no fim) | `cursor.c` e `util.c` (`ResetWrap`) |

O que divergia e foi corrigido, cada um com teste em
`core/term/src/terminal/tests/screens.rs`:

1. DECRC cancelava a quebra pendente; passou a restaurar a que o DECSC guardou.
2. DECSC e DECRC tinham um espaço só; passaram a ter um por grade e a guardar G0, G1 e SO/SI.
3. DECRC em DECOM não limitava a linha à região; passou a limitar.
4. HT cancelava a quebra pendente; HT, CHT e CBT não cancelam.
5. `?7l` cancelava a quebra pendente; só `?7l` seguido de texto sem quebra a consome.
6. `ED 3` e os parâmetros desconhecidos de ED e EL cancelavam a quebra pendente; não cancelam.

### Diferenças que ficam

- Escrever na última coluna com DECAWM desligado não deixa quebra pendente aqui. No
  xterm (`charproc.c`, `WriteText`) a bandeira também sobe nesse caso e alterna a
  cada caractere; só aparece com `?7l`, texto na margem e `?7h` logo depois, e não
  vale copiar.
- Conjuntos nacionais (NRCS) e G2/G3 não existem: o que não é `0` vira ASCII
  ([DEBT-0048](../tech-debt/0048-dec-sem-simbolos-de-controle-nem-nrcs.md)).
- Tecla que não é de rolagem também volta à tela viva; o xterm, com `scrollKey`
  falso (o padrão), não volta.

## O que ainda não existe

- Caractere largo e combinante: [DEBT-0008](../tech-debt/0008-terminal-sem-caractere-largo.md).
- Símbolos de controle do DEC, NRCS e G2/G3: [DEBT-0048](../tech-debt/0048-dec-sem-simbolos-de-controle-nem-nrcs.md).
- Histórico sem rolagem no aarch64: [DEBT-0050](../tech-debt/0050-historico-sem-rolagem-no-aarch64.md).
- Redesenho com IRQ mascarada: [DEBT-0051](../tech-debt/0051-redesenho-da-tela-com-irq-mascarada.md).
- Argumentos para o programa: [DEBT-0130](../tech-debt/0130-run-sem-argumentos-para-o-programa.md).
- Fila de saída de segundo plano que não descarta ao encher e saída de fundo na tela alternativa: [DEBT-0150](../tech-debt/0150-fila-de-saida-de-segundo-plano-descarta-ao-encher.md), [0151](../tech-debt/0151-saida-de-segundo-plano-entra-na-tela-alternativa.md).
