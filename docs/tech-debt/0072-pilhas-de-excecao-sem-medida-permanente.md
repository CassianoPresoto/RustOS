---
id: DEBT-0072
title: Pilhas de exceção (IST e de falhas) sem medida permanente de pico
status: Open
date: 2026-11-18
trigger: A 4f (medidas da fase), ou o primeiro `#DF`, NMI, `#MC` ou `#DB` real fora do autoteste
paid_date:
---

# DEBT 0072 — Pilhas de exceção (IST e de falhas) sem medida permanente de pico

## O que falta

A marca d'água cobre as pilhas das threads e a do boot, não as 4 pilhas IST de 16 KiB por CPU (x86_64, a IST4 é do `#DB` desde a 4a) nem a pilha de falhas de 16 KiB por CPU (aarch64). Medidas uma vez, com uma pintura descartável: IST sem uso nas faltas, no `#BP` e no pânico (pico zero), e a de falhas do aarch64 em 2,3 KiB (`dev`) e 1,9 KiB (`release`) no relatório de uma falta fatal.

## Por que foi aceito

Pintar 48 KiB por CPU no x86_64 e 16 KiB no aarch64 custa tempo de boot em todas as CPUs, para pilhas que quase não são usadas.

## Gatilho

A 4f, que mede a fase, ou o primeiro `#DF`, NMI, `#MC` ou `#DB` real fora do autoteste. A 4a
acrescentou a IST4 do `#DB` ([SDD 17](../sdd/17_processos_isolamento_e_syscalls.md#exceções-vindas-do-anel-3)),
exercida pelo caso `fault_trap` do autoteste de usuário, que mata o processo com o quadro já copiado
para a pilha da tarefa; o pico da IST4 não é medido.

## Como pagar

Pintar com `Stack::paint` onde `prepare_secondary` e `exception::install` mapeiam (`kernel/src/platform/`) e somar as pilhas à linha `stacks:`; a IST4 entra junto.
