---
id: DEBT-0003
title: SpinLock não mascara interrupção
status: Open
date: 2026-11-17
trigger: O primeiro handler de interrupção que pegue um lock, inclusive o do console
paid_date:
---

# DEBT 0003 — `SpinLock` não mascara interrupção

## O que falta

`lib/sync::SpinLock` só gira no `AtomicBool`. Se uma interrupção chegar com o lock
na mão e o handler tentar o mesmo lock, a CPU trava girando para sempre. O console
global (`kernel/src/console.rs`) é um `SpinLock`.

## Por que foi aceito

Hoje o kernel roda com interrupções mascaradas do início ao fim: não há handler,
então não há reentrada.

## Gatilho

O primeiro handler de interrupção que pegue um lock — o mais provável é um
`kprintln!` dentro do handler de timer ou de exceção, na fase 1.

## Como pagar

Uma porta `hal::InterruptGuard` (salvar e mascarar, restaurar no `Drop`)
implementada em `arch/`, e um `IrqSpinLock` no `kernel/` que a combine com o
`SpinLock`. `lib/sync` continua sem conhecer a ISA.
