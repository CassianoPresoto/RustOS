---
id: DEBT-0086
title: Casos de falha de usuário num blob do kernel, não em `user/tests/*`
status: Open
date: 2026-11-18
trigger: A 4d precisar do teste de `xmm0`/`v0` entre processos, ou o quarto caso novo de usuário que peça `asm!`
paid_date:
---

# DEBT 0086 — Casos de falha de usuário num blob do kernel, não em `user/tests/*`

## O que falta

Os casos da 4a (`hello`, leitura de página do kernel, instrução privilegiada, `TF`, `sp` inválido, flags e laço
de syscall nula) continuam num blob de `global_asm!` em `arch/<isa>/src/user_blob.rs`, copiado para uma página
de usuário, atrás da feature `selftest`. O SDD 17 os previa em `user/tests/*`, no initramfs. Só o `big`
(carregador e `code-model=small`) é programa de teste de verdade.

## Por que foi aceito

Cada caso de falha exige `asm!` (`hlt`, `popf`, `sp` inválido, `brk`) e o usuário em Rust seguro não o tem:
levá-los pede um crate `arch/<isa>-user` de teste com os stubs, só para repetir o que o blob já cobre, nas duas
ISAs, sem mudar o que se confere.

## Gatilho

A 4d precisar do teste de `xmm0`/`v0` entre processos (que também exige `asm!`), ou o quarto caso novo de usuário
que peça `asm!`.

## Como pagar

Um crate `arch/<isa>-user` de teste (ou a feature `selftest` dele) com os stubs de cada caso, programas
`user/tests/*` que os chamam, e o blob e os `Case` saem do `arch/<isa>` e do `kernel/`.
