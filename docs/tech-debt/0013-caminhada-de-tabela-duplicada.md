---
id: DEBT-0013
title: Caminhada de tabela de página duplicada entre as ISAs
status: Open
date: 2026-11-17
trigger: A terceira ISA, ou a primeira mudança no algoritmo de caminhada de uma delas
paid_date:
---

# DEBT 0013 — Caminhada de tabela de página duplicada entre as ISAs

## O que falta

`arch/x86_64/src/walk.rs` e `arch/aarch64/src/walk.rs` fazem a mesma caminhada de
4 níveis: achar a entrada, criar a tabela intermediária zerada e recusar página
grande no caminho. Só muda a codificação do descritor. A correção feita numa não
chega na outra.

A fase 2 acrescentou a folha de 2 MiB e a caminhada de conferência (`walk`) nas duas
cópias, no mesmo molde: o gatilho abaixo disparou e a dívida continua aberta por decisão
da fase (a tarefa mandou não pagá-la). Quem mexer na caminhada de novo paga.

## Por que foi aceito

Promover a caminhada a `core/` é crate novo ou porta nova no `hal`. Com duas ISAs
e o algoritmo estável, a cópia custa menos que a decisão.

## Gatilho

A terceira ISA, ou a primeira mudança no algoritmo de caminhada de uma delas.

## Como pagar

Um trait de formato de descriptor no `hal`, com a caminhada genérica num crate de
`core/`, que é decisão do caso 1 do `AGENTS.md`. Cada ISA fica só com a
codificação.
