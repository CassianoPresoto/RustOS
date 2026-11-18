---
id: DEBT-0052
title: Afinidade por tarefa só no `spawn`
status: Open
date: 2026-11-17
trigger: A primeira syscall de afinidade (`sched_setaffinity`), na fase 4
paid_date:
---

# DEBT 0052 — Afinidade por tarefa só no `spawn`

## O que falta

`Spawn::affinity` fixa a máscara de CPUs de uma tarefa quando ela nasce. Não há como mudar a máscara de uma tarefa viva: migrar uma tarefa que está **correndo** para fora da máscara nova exige parar a CPU dela (IPI) ou esperar a próxima troca.

## Por que foi aceito

O kernel não tem interface de afinidade para o mundo de fora, e as tarefas do kernel que precisam de CPU fixa (as medidas do `bench`) a decidem ao criar.

## Gatilho

A primeira syscall de afinidade (`sched_setaffinity`), na fase 4.

## Como pagar

Gravar a máscara nova, e se a tarefa está correndo fora dela, mandar o IPI de reescalonar à CPU dela e migrá-la na próxima troca, como o `migration_cpu_stop` do Linux; teste de duas CPUs com a tarefa trocando de máscara em laço.
