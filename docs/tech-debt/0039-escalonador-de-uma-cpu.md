---
id: DEBT-0039
title: Escalonador de uma CPU, sem fila por CPU nem balanceamento
status: Open
date: 2026-11-17
trigger: O primeiro núcleo secundário rodar tarefas, na fase 3
paid_date:
---

# DEBT 0039 — Escalonador de uma CPU

## O que falta

`Eevdf`, `Fifo`, `RoundRobin` e `Composite` ([SDD 14](../sdd/14_escalonador.md)) são
uma fila de uma CPU, sem lock interno. O kernel liga uma só instância, o `Composite` de
`task.rs`, e não há uma por CPU,
migração de tarefa, escolha de CPU ao acordar nem balanceamento.

## Por que foi aceito

O núcleo da política (lag, prazo, teto de tempo real) é o mesmo por CPU e se testa
inteiro com uma só. O Linux também mantém uma fila por CPU e balanceia por cima
(`sched-domains`). Desenhar o balanceamento antes de haver um segundo núcleo
rodando tarefa seria chutar.

## Gatilho

O primeiro núcleo secundário rodar tarefa (SMP da fase 3, pedido MP do Limine).

## Como pagar

Uma instância de `Composite` por CPU, atrás de lock por CPU (`IrqLock`), e uma política
de escolha de CPU no `wake` e no `add`. Migrar uma tarefa é `remove` numa fila e `add`
na outra, e o lag precisa viajar com ela (`Eevdf::lag_ns`, o `PLACE_REL_DEADLINE` do
Linux): hoje a tarefa nova entra com lag zero.
