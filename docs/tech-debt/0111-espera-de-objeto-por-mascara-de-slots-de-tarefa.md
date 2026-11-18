---
id: DEBT-0111
title: A espera de objeto é uma máscara de slots de tarefa, sem a retirada da lista de espera na morte da thread
status: Open
date: 2026-11-18
trigger: O `thread_create` ou o `process_kill` (parte 3 da 4c) poder matar uma thread bloqueada em `object_wait_one`, ou o `MAX_TASKS` passar de 64
paid_date:
---

# DEBT 0111 — A espera de objeto é uma máscara de slots de tarefa, sem a retirada da lista de espera na morte da thread

## O que falta

Cada objeto guarda um `AtomicU64` com um bit por slot de tarefa (`Waiters`), no lugar da lista de espera de cada objeto que o SDD 17 descreve. Um `const _` impede `MAX_TASKS` acima de 64. A thread que sai de `object_wait_one` retira o próprio bit; a que morre bloqueada (ainda não existe: um processo tem uma thread e só ela o mata) o deixaria, e um aviso acordaria o slot reusado por outra tarefa.

## Por que foi aceito

Sem alocação, sem lista, e acordar roda com qualquer lock preso. Nenhuma chamada da parte 1 mata uma thread bloqueada de fora.

## Gatilho

O `thread_create` ou o `process_kill` entrarem (a morte da thread bloqueada passa a existir), ou `MAX_TASKS` passar de 64 (o `const _` quebra o build).

## Como pagar

Na morte do processo, passo 2 do SDD 17: retirar o bit do slot de todo objeto em que a thread espera (o objeto fica no frame de `object_wait_one`, então basta a thread registrar o que espera no slot da tarefa), ou passar a máscara para uma lista intrusiva por objeto.
