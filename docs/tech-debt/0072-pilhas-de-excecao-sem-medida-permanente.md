---
id: DEBT-0072
title: Pilhas de exceção (IST e de falhas) sem medida permanente de pico
status: Open
date: 2026-11-18
trigger: A 4a acrescentar a IST do `#DB` ou fazer o aarch64 usar a pilha de falhas só no estouro
paid_date:
---

# DEBT 0072 — Pilhas de exceção (IST e de falhas) sem medida permanente de pico

## O que falta

A marca d'água cobre as pilhas das threads e a do boot, não as 3 pilhas IST de 16 KiB por CPU (x86_64) nem a pilha de falhas de 16 KiB por CPU (aarch64). Medidas uma vez, com uma pintura descartável: IST sem uso nas faltas, no `#BP` e no pânico (pico zero), e a de falhas do aarch64 em 2,3 KiB (`dev`) e 1,9 KiB (`release`) no relatório de uma falta fatal.

## Por que foi aceito

Pintar 48 KiB por CPU no x86_64 e 16 KiB no aarch64 custa tempo de boot em todas as CPUs, para pilhas que quase não são usadas.

## Gatilho

A 4a mudar o papel dessas pilhas (IST4 do `#DB`, pilha de falhas só no estouro, [SDD 17](../sdd/17_processos_isolamento_e_syscalls.md#o-que-a-fase-entrega-por-fatia)), ou o primeiro `#DF` ou NMI real.

## Como pagar

Pintar com `Stack::paint` onde `prepare_secondary` e `exception::install` mapeiam (`kernel/src/platform/`) e somar as pilhas à linha `stacks:`.
