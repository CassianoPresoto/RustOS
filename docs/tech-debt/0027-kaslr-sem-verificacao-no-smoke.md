---
id: DEBT-0027
title: KASLR sem verificação entre boots no smoke
status: Open
date: 2026-11-17
trigger: O primeiro PR que mexer no linker script, no `-pie` ou no `limine.conf`
paid_date:
---

# DEBT 0027 — KASLR sem verificação no smoke

## O que falta

Que a base do kernel muda entre boots foi medido à mão (5 boots por ISA, SDD 13), mas
nenhum comando confere isso. O `make smoke` sobe uma vez e só exige `boot ok`: se o
`kaslr: yes` sumir do `limine.conf`, ou o PIE quebrar, o kernel continua subindo em
`0xffffffff80000000` e nada falha.

## Por que foi aceito

Um boot a mais no smoke custa 2 a 5 s por ISA, e a regressão só aparece em mexida no
linker script, no PIE ou no `limine.conf`, que são raras.

## Gatilho

O primeiro PR que mexer em `kernel/linker/*.ld`, no `-pie` de `kernel/build.rs` ou no
`limine.conf` de `xtask/src/image.rs`.

## Como pagar

No `xtask smoke`, subir duas vezes por ISA, ler `[rustos] kernel base:` das duas
saídas e falhar se forem iguais (com 2^19 posições possíveis, a chance de empate é
desprezível). Atenção ao tempo do smoke.
