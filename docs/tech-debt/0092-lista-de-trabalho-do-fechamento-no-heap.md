---
id: DEBT-0092
title: Lista de trabalho do fechamento de handles sem teto e com `push` que não falha
status: Open
date: 2026-11-18
trigger: O kernel da 4c implementar `Worklist` sobre o heap
paid_date:
---

# DEBT 0092 — Lista de trabalho do fechamento de handles sem teto e com `push` que não falha

## O que falta

`close` e `close_pending` percorrem uma `Worklist` (sem recursão). A morte de uma ponta empurra até
128 handles, e cada um pode matar outra ponta: a lista só é limitada pelo número de pontas vivas. A
trait exige `push` infalível; sobre um `Vec`, falta de memória no heap vira pânico no meio do
fechamento.

## Por que foi aceito

O `core/object` é puro e não decide onde a lista mora. Na 4c o kernel a implementa.

## Gatilho

O kernel da 4c implementar `Worklist` sobre o heap.

## Como pagar

Lista intrusiva (cada ponta morre uma vez e guarda o elo) ou reserva prévia por `try_reserve` no
`channel_create`, de modo que o fechamento nunca aloque.
