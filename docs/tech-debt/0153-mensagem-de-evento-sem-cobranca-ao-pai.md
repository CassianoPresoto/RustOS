---
id: DEBT-0153
title: A mensagem do evento do fim não é cobrada ao pai
status: Open # Open | Paid | Dropped
date: 2026-11-23
trigger: O primeiro programa de usuário que cria processos com ponta de evento em laço e não lê os eventos (o heap de cada mensagem sai da conta do teto)
paid_date:
---

# DEBT 0153 — A mensagem do evento do fim não é cobrada ao pai

## O que falta

A mensagem de `ProcessEvent` (56 bytes, mais 64 de cabeçalho do kernel) é escrita pelo kernel,
então nenhum processo a paga: a regra de "mensagem em fila, cobrada ao processo que escreveu"
do [SDD 17](../sdd/17_processos_isolamento_e_syscalls.md) não se aplica a ela. O custo fica
limitado pelo número de pontas de eventos que o pai consegue ter vivas (uma mensagem por par,
e o par custa 8 KiB da conta dele), mas fora do teto em bytes.

## Por que foi aceito

A conta é pequena e limitada (algumas dezenas de mensagens de ~120 bytes no pior caso, por
pai) e cobrar o kernel exigiria escolher um dono que não existe. Cobrar ao pai que criou o
par é possível, mas muda o contrato de quem paga por uma ponta.

## Gatilho

O primeiro programa de usuário que cria processos com ponta de evento em laço e não lê os
eventos: aí a conta de heap sai do teto de bytes que o SDD 17 promete.

## Como pagar

Cobrar a mensagem do evento ao dono da ponta que a recebe (`Bytes::charged` com a conta do pai),
em `Process::report` (`kernel/src/process.rs`), e incluir o caso no `procs`.
