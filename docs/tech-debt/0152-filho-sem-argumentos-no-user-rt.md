---
id: DEBT-0152
title: `user/rt` não passa argumentos ao filho que cria (`process_create` sem `argv`)
status: Open
date: 2026-11-23
trigger: O primeiro programa de `user/` que crie um processo filho com argumentos
paid_date:
---

# DEBT 0152 — `user/rt` não passa argumentos ao filho que cria (`process_create` sem `argv`)

## O que falta

O `run` do shell passa `argv` ao programa (o kernel monta a pilha com o nome e os argumentos). Quem cria
um filho pelo `user/rt` (`spawn.rs`, com `process_create`) monta a pilha do filho com `argv` vazio
(`InitialStack::build(top, &info, &[])`): o filho sempre vê `argc = 0`.

## Por que foi aceito

Nenhum programa de `user/` cria filho hoje, e o `process_create` veio da parte 3 da 4c sem API de
argumentos. Levar `argv` ao filho pede uma função nova no `rt` e, se o `process_create` precisar
transportá-lo, uma mudança de syscall, que é contrato de fora (regra 6 do `AGENTS.md`).

## Gatilho

O primeiro programa de `user/` que crie um processo filho com argumentos.

## Como pagar

`spawn::create` do `user/rt` recebe o `argv` e `Child::map_stack` o passa a `InitialStack::build` (que já aceita, com os tetos
`ARGS_MAX` e `ARGS_BYTES_MAX`). Se a criação pelo kernel precisar do `argv`, a mudança passa pela decisão
de syscall antes de qualquer código.
