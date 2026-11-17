---
id: DEBT-0036
title: Escalonador com estado de troca global
status: Open
date: 2026-11-17
trigger: O primeiro núcleo secundário subir, na fase 3
paid_date:
---

# DEBT 0036 — Escalonador com estado de troca global

## O que falta

`SCHED` é um `IrqLock` global, e `IN_IRQ`, o contador `locks_held`, `ARMED_NS`, `INPUT_WAITER` e a pilha de falhas do aarch64 (`SP_EL0`) são um só para a máquina. Os `Context`s ficam fora do lock e a troca os usa com o lock solto, o que só é seguro numa CPU.

## Por que foi aceito

O kernel roda em uma CPU até o SMP (DEBT-0011 e DEBT-0018 têm o mesmo gatilho).

## Gatilho

O primeiro núcleo secundário subir, na fase 3.

## Como pagar

Estado por CPU: fila de execução, `current`, `IN_IRQ`, `locks_held`, timer de um tiro e pilha de falhas. A tarefa que sai só fica visível a outra CPU depois do `switch` terminar (marcar `Ready` do lado de quem entra), e o acordar entre CPUs usa IPI.
