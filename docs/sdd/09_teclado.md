# SDD 09 — Teclado

> **O que este capítulo responde:** como uma tecla do PC vira a `Key` que o
> terminal codifica em bytes, e de onde vem cada tabela.

## O caminho de uma tecla

| Passo | Quem | O que faz |
| --- | --- | --- |
| 1 | `drivers/i8042` (`I8042::read_event`) | Lê o byte do controlador pela porta `hal::RegisterBus` e o entrega ao decodificador |
| 2 | `drivers/i8042` (`Set1Decoder`) | Decodifica o conjunto 1 traduzido (make/break, prefixos `E0` e `E1`) em `hal::KeyEvent` |
| 3 | `core/term` (`Keyboard::feed`) | Aplica layout, modificadores e teclas mortas; devolve `Key` |
| 4 | `core/term` (`Terminal::encode_key`) | Transforma `Key` em bytes VT/xterm |

`hal::KeyCode` é o código de uso do HID USB, página 0x07 ([HID Usage Tables 1.4,
seção 10](https://usb.org/document-library/hid-usage-tables-14)). O driver não
conhece layout; o `Keyboard` não conhece o dispositivo. O `kernel/` liga os dois
(`PortBus` com base 0x60: offset 0 é o dado, offset 4 é status/comando).

## O controlador (`drivers/i8042`)

Comandos, bits de status e de configuração vêm de
`include/linux/i8042.h` e da sequência de `drivers/input/serio/i8042.c` do Linux.

| Passo do `init` | Porta | Valor |
| --- | --- | --- |
| Desliga teclado e mouse | `0x64` | `0xAD`, `0xA7` |
| Esvazia a saída | `0x60` | até 16 leituras; saída que não esvazia é `StuckOutput` |
| Autoteste | `0x64` | `0xAA`, resposta `0x55`; até 5 tentativas |
| Lê a configuração | `0x64` | `0x20` |
| Escreve a configuração quieta | `0x64`, `0x60` | `0x60`, valor com clock desligado, sem IRQ, bit 6 (tradução) ligado |
| Liga a porta 1 | `0x64` | `0xAE` |
| Escreve a configuração final | `0x64`, `0x60` | `0x60`, IRQ 1 (bit 0) ligada, clock da porta 1 ligado |

A porta 2 (mouse) fica desligada. Toda espera tem limite (100 000 consultas do
status): sem resposta, `I8042Error::Timeout`.

## O decodificador

O mapa conjunto 1 → HID foi conferido byte a byte, por script, contra o Linux:
`atkbd_unxlate_table` e `atkbd_set2_keycode` (`drivers/input/keyboard/atkbd.c`)
até o código de tecla, e `hid_keyboard` (`drivers/hid/hid-input.c`) do código de
tecla ao uso do HID. Zero divergências nas 112 teclas mapeadas; ficam de fora as
de mídia, energia e as japonesas além de `International1` a `International5`.

- `E0 2A`/`E0 36`/`E0 AA`/`E0 B6` (shift falso do Print Screen) não geram evento.
- `E1` (Pause) consome os 5 bytes seguintes e não gera evento.
- Byte com bit de paridade ou timeout, e byte da porta 2, são descartados.
- `set 1` `0x73` é a tecla `/?` do ABNT2 (`International1`, HID `0x87`) e `0x7E` é
  o `.` do teclado numérico do ABNT2 (`Keypad Comma`, HID `0x85`).

## Os layouts (`core/term::keyboard`)

`Layout::Us` e `Layout::BrAbnt2`. Cada um é uma tabela fixa, ordenada pelo HID,
com quatro níveis por tecla: base, Shift, AltGr, AltGr+Shift. Sem alloc.

| Tabela | Fonte |
| --- | --- |
| `US` | `symbols/us` do [xkeyboard-config](https://gitlab.freedesktop.org/xkeyboard-config/xkeyboard-config) |
| `ABNT2` | `symbols/br`, variante `abnt2`, mais `symbols/latin` nas teclas que o `br` não redefine |

No `ABNT2` só entram símbolos que existem em Latin-1 (a fonte de `core/gfx` cobre
U+0020 a U+00FF); os demais níveis do xkb (letras gregas, setas, `ŧ`) ficam sem
símbolo. AltGr numa tecla sem nível 3 digita o caractere base, como o xkb.
AltGr é o Alt direito só no `ABNT2`; no `US` é Alt.

Regras do `Keyboard`:

- **Ctrl+tecla** vira `Key::Ctrl(c)` com `c` ASCII minúsculo; fora de ASCII não
  gera nada.
- **Alt+caractere** vira `Key::Escape` e depois o caractere (prefixo ESC do xterm).
- **Caps Lock** troca o nível só de letras (a–z e `ç`); com Shift, volta ao
  minúsculo. Alterna uma vez por pressionamento físico.
- **Teclado numérico** sempre com Num Lock ligado (DEBT-0009); no `ABNT2` a tecla
  `.` do bloco digita `,` (`kpdl(comma)`) e a extra (`0x85`) digita `.`.

### Teclas mortas

`´` (tecla à direita do `P`), `` ` `` (Shift+essa), `~` (tecla à direita do `ç`), `^` (Shift+essa), `¨`
(Shift+6) e AltGr+`ç` (`´`). A combinação sai de tabelas de pares em
`core/term/src/keyboard/layout.rs`:

| Acento | Combina com |
| --- | --- |
| `´` | a e i o u y, maiúsculas, e `c` → `ç` (como o ABNT2 do Windows) |
| `` ` `` | a e i o u, maiúsculas |
| `~` | a o n, maiúsculas |
| `^` | a e i o u, maiúsculas |
| `¨` | a e i o u y, maiúsculas, exceto `Ÿ` (fora do Latin-1) |

Tecla morta + espaço digita o próprio acento (`´`, `` ` ``, `~`, `^`, `¨`). Mais de
um caractere por tecla (acento e letra que não combinam) sai em fila de 4 posições:
`feed` devolve a primeira, `next_queued` as outras, e o próximo `feed` também
esvazia a fila. A mesma tecla morta duas vezes digita o acento uma vez. Tecla
que não é caractere (Enter, setas, Ctrl+letra) digita o acento pendente antes.

## O que ainda não existe

Reset e LEDs do teclado, Num Lock e teclado USB: [DEBT-0009](../tech-debt/0009-teclado-sem-comandos-ao-dispositivo.md).
Interrupção ligada ao kernel (IRQ 1) é da fase 1 do [roadmap](08_roadmap.md).
