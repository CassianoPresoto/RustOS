---
id: DEBT-0084
title: Programas de usuário do x86_64 sem LTO por causa do `code-model=small`
status: Open
date: 2026-11-18
trigger: O `core` pré-compilado do toolchain passar a vir com `code-model=small`, ou o `init` passar de 8 páginas de código
paid_date:
---

# DEBT 0084 — Programas de usuário do x86_64 sem LTO por causa do `code-model=small`

## O que falta

Com `-C code-model=small`, o LTO fat dos perfis `release` e `release-small` falha no x86_64: o `core`
pré-compilado do alvo (`code-model=kernel`) e o código do usuário têm o módulo de LLVM com `Code Model`
diferente. O `cargo xtask user` liga `-C lto=off` no x86_64. No aarch64 não há `code-model` e o LTO fica.

## Por que foi aceito

O `small` é o que o [SDD 17](../sdd/17_processos_isolamento_e_syscalls.md#o-que-muda-no-linker-e-no-alvo)
decidiu. Medido com o `init` na `release-small`: 17 600 bytes sem LTO contra 15 176 com o modelo `kernel`
e LTO; o código gerado nos dois modelos é do mesmo tamanho em `0x40_0000` (17 920 bytes nos dois sem
LTO), então o modelo não muda nada que o programa execute.

## Gatilho

O `core` pré-compilado do toolchain passar a vir com `code-model=small`, ou o `init` passar de 8 páginas de
código (a perda do LTO deixa de ser desprezível).

## Como pagar

Tirar o `-C code-model=small` (o `kernel` gera o mesmo código abaixo de 2 GiB) e o `-C lto=off` em
`xtask/src/user.rs`, ou compilar o `core` do usuário com `build-std` (caso 2 do `AGENTS.md`).
