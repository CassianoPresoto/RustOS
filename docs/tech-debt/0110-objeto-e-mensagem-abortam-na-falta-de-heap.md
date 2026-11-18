---
id: DEBT-0110
title: Objeto e mensagem de canal abortam o kernel na falta de heap
status: Open
date: 2026-11-18
trigger: O primeiro programa de usuário que não seja nosso, ou a contabilidade de recursos da fase 9
paid_date:
---

# DEBT 0110 — Objeto e mensagem de canal abortam o kernel na falta de heap

## O que falta

`Arc::new` dos objetos (`channel_create`, o canal de boot, o `ProcessObj`), a cópia dos bytes de uma mensagem (`Arc<[u8]>`) e a lista de trabalho do fechamento (`Vec`) chamam o alocador sem tratar a falta: o heap cheio vira `handle_alloc_error`, que é pânico. Só os slots da tabela usam `try_reserve` e respondem `NoMemory`. A cobrança de 256 pontas por processo limita o estrago de um processo, não o de vários.

## Por que foi aceito

A toolchain estável não tem `Arc::try_new`, e a medida da fase 4 não tem um programa hostil que esgote o heap. Os tetos de mensagem (1 KiB), de fila (16) e de pontas (256) limitam o que um processo aloca.

## Gatilho

O primeiro programa de usuário que não seja nosso, ou a contabilidade de recursos da fase 9 (o mesmo evento que limita o `vmo_create`), o que vier antes.

## Como pagar

Reservar com `try_reserve` o que for `Vec` e trocar o `Arc<[u8]>` da mensagem por um tipo do kernel com contagem própria e alocação fracassável; para os `Arc` de objeto, uma caixa com contagem própria. Um teste com o heap cheio que espera `NoMemory` em cada chamada.
