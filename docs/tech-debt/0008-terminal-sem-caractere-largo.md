---
id: DEBT-0008
title: Terminal sem caractere largo nem combinante
status: Open
date: 2026-11-17
trigger: O primeiro programa da fase 5 (vi, less) mostrando CJK, emoji ou acento decomposto no terminal, ou a fonte ganhar glifo de largura dupla
paid_date:
---

# DEBT 0008 — Terminal sem caractere largo nem combinante

## O que falta

`core/term` põe 1 célula em todo caractere, e descarta o combinante e o de
largura zero (`is_zero_width`). CJK e emoji ocupam 1 célula em vez de 2, e
`e` + U+0301 (texto decomposto, comum ao colar do macOS) sai como `e`. O
`core/shell` também conta 1 célula por caractere, inclusive para o combinante:
o cursor da linha editada fica 1 coluna atrás até o próximo redesenho.

A tela alternativa, o histórico de rolagem, o desenho de caixas e as paradas de
tabulação já existem ([SDD 16](../sdd/16_terminal.md)), assim como a conferência
de quebra pendente, DECSTBM e DECOM contra o xterm.

## Por que foi aceito

- **Largura:** a fonte de `core/gfx` só tem U+0020 a U+00FF e alguns símbolos, e
  nenhum glifo de largura dupla. Largura dupla hoje desenharia o substituto em 2
  células, sem ganho. Ela pede a tabela East Asian Width do Unicode (algumas
  centenas de faixas) e uma célula de continuação que o `Cell` de 8 bytes não tem.
- **Combinante:** pede um segundo ponto de código por célula, e o `Cell` tem
  exatamente 8 bytes (`size_of::<Cell>() == 8`, afirmado em `cell.rs`).

## Gatilho

O primeiro programa da fase 5 (vi, less) mostrando CJK, emoji ou acento decomposto
no terminal, ou a fonte ganhar glifo de largura dupla.

## Como pagar

- **Largura:** tabela East Asian Width (W e F) em faixas ordenadas, busca binária,
  e uma marca de continuação na célula seguinte. O `gfx` ganha o desenho de glifo
  de duas células.
- **Combinante:** decidir entre um byte a mais no `Cell` (e o custo do SDD 07 por
  célula) e uma tabela lateral de combinantes por linha.
- **Shell:** `core/shell` passa a usar a mesma largura do terminal.
