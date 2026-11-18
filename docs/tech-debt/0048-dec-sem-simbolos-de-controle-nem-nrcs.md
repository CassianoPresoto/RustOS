---
id: DEBT-0048
title: DEC Special Graphics sem os símbolos de controle, e sem NRCS, G2 nem G3
status: Open
date: 2026-11-17
trigger: O primeiro programa curses da fase 5 que desenhe ACS fora das caixas e dos blocos
paid_date:
---

# DEBT 0048 — DEC Special Graphics sem os símbolos de controle, e sem NRCS, G2 nem G3

## O que falta

- **Seis símbolos de controle** do DEC Special Graphics (`b c d e h i`: ␉ ␌ ␍ ␊ ␤ ␋,
  U+2409 a U+2424): o terminal os guarda na célula, mas a fonte de `core/gfx` não
  tem glifo e o `render` desenha o substituto (U+FFFD). Caixas, blocos, setas, `◆ ≤ ≥ π ≠` e as quatro
  linhas de varredura têm glifo.
- **NRCS, G2 e G3:** só G0 e G1 existem, e só ASCII ou DEC Special Graphics. O que
  `ESC ( C` designa e não é `0` vira ASCII; `ESC * C`, `ESC + C` e `SS2`/`SS3` são
  ignorados.

## Por que foi aceito

O programa que exige isso ainda não existe: `vi`, `less` e `top` só pedem caixas.
Os seis símbolos raramente aparecem fora de programas curses antigos.

## Gatilho

O primeiro programa curses da fase 5 que desenhe ACS fora das caixas e dos blocos.

## Como pagar

- **Símbolos:** seis glifos em `core/gfx/src/font/glyphs.txt` (U+2409, U+240A, U+240B,
  U+240C, U+240D, U+2424).
- **NRCS, G2 e G3:** `Charset` ganha as variantes pedidas (UK primeiro, por causa
  do `£`), `Terminal` ganha G2 e G3 e o `SS2`/`SS3`, e o DECSC os guarda. A fonte é
  `charsets.c` do xterm, já citada no SDD 16.
