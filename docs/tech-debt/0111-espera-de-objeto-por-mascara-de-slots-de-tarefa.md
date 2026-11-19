---
id: DEBT-0111
title: A espera de objeto é uma máscara de slots de tarefa, sem a retirada da lista de espera na morte da thread
status: Open
date: 2026-11-18
trigger: O `MAX_TASKS` passar de 64 (o `const _` quebra o build), ou um objeto cujo aviso acordar o slot reusado por outra tarefa
paid_date:
---

# DEBT 0111 — A espera de objeto é uma máscara de slots de tarefa, sem a retirada da lista de espera na morte da thread

## O que falta

Cada objeto guarda um `AtomicU64` com um bit por slot de tarefa (`Waiters`), no lugar da lista de espera de cada objeto que o SDD 17 descreve. Um `const _` impede `MAX_TASKS` acima de 64. A thread que sai de `object_wait_one` retira o próprio bit, e a que `process_kill` acorda sai da espera pelo mesmo caminho (o laço confere a morte do processo a cada volta), então a morte de thread bloqueada **não** deixa bit. Sobra o limite de 64 slots e o aviso a um slot que outra tarefa já reusou, que só acorda alguém à toa.

## Por que foi aceito

Sem alocação, sem lista, e acordar roda com qualquer lock preso. A parte 3 da 4c cobriu a morte de thread bloqueada (teste `procs`, `process_kill` de processo bloqueado no kernel).

## Gatilho

`MAX_TASKS` passar de 64 (o `const _` quebra o build).

## Como pagar

Passar a máscara para uma lista intrusiva por objeto, ou uma máscara de 128 bits.
