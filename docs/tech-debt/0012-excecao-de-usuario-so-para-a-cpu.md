---
id: DEBT-0012
title: Exceção vinda de código de usuário só para a CPU
status: Open
date: 2026-11-17
trigger: O primeiro código em anel 3 ou EL0, na fase 4
paid_date:
---

# DEBT 0012 — Exceção vinda de código de usuário só para a CPU

## O que falta

Toda exceção não recuperável trata o caso como falha do kernel: relata e para a
CPU. Não há caminho para matar só o processo culpado. No aarch64, os vetores de
EL0 (A64 e A32), FIQ e SError também só param.

## Por que foi aceito

Ainda não existe código de usuário. Toda exceção hoje é do kernel, e parar é o
certo.

## Gatilho

O primeiro código em anel 3 ou EL0, na fase 4.

## Como pagar

Distinguir a origem pelo `CS` salvo (x86_64) ou pelo grupo do vetor (aarch64).
Exceção de usuário vira sinal ou término do processo, e o kernel segue. SError e
machine check continuam fatais e com relatório.
