---
id: DEBT-0008
title: Terminal sem tela alternativa, rolagem para trás nem caractere largo
status: Open
date: 2026-11-17
trigger: O primeiro programa de tela cheia (vi, less, top) rodando no terminal, na fase 5
paid_date:
---

# DEBT 0008 — Terminal sem tela alternativa, rolagem para trás nem caractere largo

## O que falta

`core/term` ignora:

- **Modos de tela:** a tela alternativa (`?1049`, `?47`, `?1047`), o histórico de
  rolagem e o `ED 3`.
- **Desenho de caixas:** a troca de conjunto de caracteres (`ESC ( 0`). Caixas
  desenhadas por ACS saem como letras.
- **Tabulação:** paradas fixas a cada 8 colunas, sem HTS/TBC.
- **Unicode:** caractere largo (CJK, emoji) ocupa 1 célula, e o combinante é
  descartado. O `core/shell` conta 1 célula por caractere, inclusive para o
  combinante: com texto decomposto (`e` + U+0301, comum ao colar do macOS), o
  cursor da linha editada fica 1 coluna atrás até o próximo redesenho.

O comportamento de pending wrap, DECSTBM e DECOM veio do conhecimento do agente
sobre o xterm. Está coberto por teste, mas não foi conferido linha a linha contra
o `ctlseqs`.

## Por que foi aceito

O único escritor do terminal hoje é o kernel e o shell embutido, que só usam
texto, cor, cursor e limpeza de tela.

## Gatilho

O primeiro programa de tela cheia (vi, less, top) rodando no terminal, na fase 5.

## Como pagar

- **Tela alternativa:** uma segunda grade alocada pelo kernel e entregue ao
  `Terminal`.
- **Histórico:** um anel de linhas antigas com capacidade fixa e custo escrito no
  SDD 07.
- **Desenho de caixas:** o mapa DEC Special Graphics para os glifos de caixa que a
  fonte já tem.
- **Caractere largo:** largura pela tabela East Asian Width do Unicode.
- **Conferência:** um teste por sequência conferido contra o `ctlseqs` do xterm.
