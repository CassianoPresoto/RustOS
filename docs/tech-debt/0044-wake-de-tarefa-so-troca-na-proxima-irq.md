---
id: DEBT-0044
title: Wake feito em contexto de tarefa só troca na próxima IRQ
status: Open
date: 2026-11-17
trigger: O primeiro programa interativo que medir a latência de acordar entre tarefas (áudio ou jogo)
paid_date:
---

# DEBT 0044 — Wake feito em contexto de tarefa só troca na próxima IRQ

## O que falta

O `PREEMPT_SHORT` (a preempção por acordar de fatia curta) vale só para o wake que vem de
IRQ, onde `preempt` roda no fim. `unpark` e `spawn` chamados de uma tarefa deixam a
acordada pronta e a corrente segue até ceder a CPU ou até a próxima IRQ do timer.

Com vários núcleos, o acordar que cai numa **outra** CPU já preempta na hora: o IPI de reescalonar
faz a CPU destino trocar ao fim da IRQ. Só o acordar na CPU de quem chama segue esperando.

## Por que foi aceito

O `bench` e o shell não medem acordar entre tarefas, e o ponto de preempção fora da IRQ
pede decidir a regra (a corrente cede na hora ou no fim da fatia) com um caso real.

## Gatilho

O primeiro programa interativo que medir a latência de acordar entre tarefas, como um
produtor de áudio ou o laço de um jogo.

## Como pagar

Um ponto de preempção no fim de `unpark` e de `spawn` quando chamados fora da IRQ
(`needs_resched` e `schedule`, com a interrupção mascarada); medir o jitter do acordar
antes e depois, com o `bench`.
