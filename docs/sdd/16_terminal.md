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
| Grade alternativa | `with_alternate(alt)` | o mesmo da principal | `?47`, `?1047` e `?1049` não trocam de tela |
| Histórico | `with_history(cells)` | `history_cells_needed(cols, linhas)` | `Shift+PageUp` não rola nada |

`new` recusa `rows > MAX_ROWS` (256) e `cols > MAX_COLS` (512): são os tamanhos dos
bitmaps de linhas sujas e de paradas de tabulação. O kernel
(`kernel/src/screen.rs`) aloca as três com `Memory::alloc_slice` e corta `cols` e
`rows` nesses limites. O custo em bytes está no [SDD 07](07_desempenho_e_recursos.md).

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
| `` ` `` | ◆ U+25C6 | não | `t` | ├ U+251C | sim |
| `a` | ▒ U+2592 | sim | `u` | ┤ U+2524 | sim |
| `b` `c` `d` `e` `h` `i` | ␉ ␌ ␍ ␊ ␤ ␋ (U+2409 a U+2424) | não | `v` | ┴ U+2534 | sim |
| `f` | ° U+00B0 | sim | `w` | ┬ U+252C | sim |
| `g` | ± U+00B1 | sim | `x` | │ U+2502 | sim |
| `j` | ┘ U+2518 | sim | `y` `z` | ≤ ≥ U+2264 U+2265 | não |
| `k` | ┐ U+2510 | sim | `{` `\|` | π ≠ U+03C0 U+2260 | não |
| `l` | ┌ U+250C | sim | `}` | £ U+00A3 | sim |
| `m` | └ U+2514 | sim | `~` | · U+00B7 | sim |
| `n` | ┼ U+253C | sim | `o` `p` `r` `s` | ⎺ ⎻ ⎼ ⎽ U+23BA a U+23BD | não |
| `q` | ─ U+2500 | sim | | | |

## O teclado e o kernel

`Keyboard` devolve `Key::ScrollBack` para Shift+PageUp e `Key::ScrollForward` para
Shift+PageDown; `encode_key` não gera byte para elas, então o programa não as vê.
`Screen::key_event` rola a visão em meia tela (`rows / 2`) e redesenha ali mesmo,
porque sem byte o laço do shell não chama o `render`. Esse desenho roda na IRQ do
teclado. Qualquer outra tecla volta à tela viva antes de virar byte.

O shell tem dois comandos de demonstração: `lines` desenha uma tabela com o DEC
Special Graphics e `alt` / `alt off` entra e sai da tela alternativa.

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
- Conjuntos nacionais (NRCS) e G2/G3 não existem: o que não é `0` vira ASCII.
- Tecla que não é de rolagem também volta à tela viva; o xterm, com `scrollKey`
  falso (o padrão), não volta.

## O que ainda não existe

Caractere largo e combinante: [DEBT-0008](../tech-debt/0008-terminal-sem-caractere-largo.md).
