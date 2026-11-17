---
id: DEBT-0045
title: `debug_assert` de lock preso ao ceder a CPU some em `release`
status: Open
date: 2026-11-17
trigger: O primeiro travamento em `release` que não reproduza em `dev`
paid_date:
---

# DEBT 0045 — `debug_assert` de lock preso ao ceder a CPU some em `release`

## O que falta

`schedule_at` confere com `debug_assert_eq!(locks_held(), 0)` que nenhum `IrqLock` está
preso ao trocar de contexto. Em `release` a conferência some, e um lock preso na troca vira
deadlock silencioso, sem mensagem na serial.

## Por que foi aceito

Custa um acesso ao contador em cada troca, e a troca é o caminho que o SDD 07 mede (DEBT-0042).
O `dev` e o `smoke` pegam o erro de escrita; o que falta é o erro que só aparece com a
otimização.

## Gatilho

O primeiro travamento em `release` que não reproduza em `dev`.

## Como pagar

Uma checagem barata também em `release` (um `if` com pânico, ou só o contador lido uma vez
por troca); medir a troca antes e depois com o `bench`, nos dois alvos.
