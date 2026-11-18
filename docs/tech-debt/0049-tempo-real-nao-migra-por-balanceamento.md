---
id: DEBT-0049
title: Tempo real não migra por balanceamento entre CPUs
status: Open
date: 2026-11-17
trigger: A primeira tarefa de tempo real do kernel (a classe FIFO hoje não tem uso), ou o primeiro RT que espere atrás de outro numa CPU com outra ociosa
paid_date:
---

# DEBT 0049 — Tempo real não migra por balanceamento entre CPUs

## O que falta

O pull de uma CPU ociosa (`Composite::pull_candidate`) só considera tarefa justa. Uma tarefa
FIFO fica na fila onde foi colocada: só `spawn` e `unpark` escolhem CPU para ela, e uma
FIFO pronta atrás de outra de prioridade maior numa CPU não é puxada por outra CPU
ociosa. O Linux tem push/pull de RT (`rt.c`), que mantém as N maiores prioridades nas N CPUs.

## Por que foi aceito

Nenhuma tarefa do kernel usa a classe de tempo real ainda ([SDD 14](../sdd/14_escalonador.md)), e
a política de push/pull de RT é outra, com prioridade e não com lag. Fazer sem um caso real
seria chutar.

## Gatilho

A primeira tarefa de tempo real do kernel, ou o primeiro RT que espere atrás de outro numa CPU
enquanto outra está ociosa.

## Como pagar

`pull_candidate` do `Fifo` (a de maior prioridade esperando que a CPU dona não vá rodar) e um
`detach`/`add_with_lag` no pull, com teste de duas filas em `core/sched/src/migration.rs`; mais o
push quando uma FIFO de prioridade maior acorda numa CPU ocupada por outra menor.
