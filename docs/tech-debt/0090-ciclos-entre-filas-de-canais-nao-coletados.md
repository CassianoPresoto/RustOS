---
id: DEBT-0090
title: Ciclos entre filas de canais não são coletados
status: Open
date: 2026-11-18
trigger: O primeiro programa que produza o ciclo entre duas filas, ou os grupos de recursos da fase 9
paid_date:
---

# DEBT 0090 — Ciclos entre filas de canais não são coletados

## O que falta

A ponta de canal morre quando todo handle dela está dentro da fila dela (SDD 17, "Destruição de
um canal"). Um handle de B1 na fila de B2 e um de B2 na fila de B1 mantêm as duas pontas vivas
para sempre: nenhum handle em tabela as alcança e a contagem nunca baixa. O modelo de teste do
`core/object` confirma: depois de fechar todas as tabelas, sobram contagens só nesses ciclos.

Medido por revisor na parte 1 da 4c: um processo hostil que enche as 256 pontas chega a **17,7 MiB** de pico, e
127 ciclos entre canais (cerca de 1,5 mil syscalls) deixam **18,3 MiB vazados para sempre**: a cobrança de pontas
limita o número de pontas, não os bytes das filas, e a cobrança some com o processo que morre.

## Por que foi aceito

Coletar por alcançabilidade percorre todos os canais, um coletor no kernel. A fase 4 limita os
pontas de canal por processo (`MAX_CHANNEL_ENDS`, 256, cobrada ao criador; feita na parte 1 da 4c e
testada no programa `handles`) e nenhum programa da fase produz o ciclo.

## Gatilho

O primeiro programa que crie o ciclo, ou os grupos de recursos da fase 9, o que vier antes.

## Como pagar

Marcar a partir dos handles em tabela e varrer os canais não marcados, ou contar a alcançabilidade
por grupo de recursos na fase 9. O teto em **bytes** por processo (filas e mensagens) já limita cada ciclo à conta do criador
(`PROCESS_BYTES_MAX`, 512 KiB), mas o ciclo cobrado a quem morreu segue vivo.
