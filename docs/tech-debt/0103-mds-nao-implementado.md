---
id: DEBT-0103
title: MDS sem limpeza de buffers na volta ao usuário (`VERW`)
status: Open
date: 2026-11-18
trigger: O primeiro boot em metal
paid_date:
---

# DEBT 0103 — MDS sem limpeza de buffers na volta ao usuário (`VERW`)

## O que falta

A linha `mitigations:` diz `mds off (not implemented)` na CPU Intel sem `MDS_NO` e fora da lista do
Linux (Skylake e anteriores, por exemplo). O kernel não executa `VERW` antes do `SYSRET`/`IRETQ`.
O aarch64 não tem a vulnerabilidade.

## Por que foi aceito

A QEMU e o HVF não modelam a limpeza; só o metal prova que o `VERW` limpa. O custo de um `VERW` por
syscall tem de ser medido em hardware.

## Gatilho

O primeiro boot em metal.

## Como pagar

`VERW` com o operando de memória do segmento de dados do kernel na saída para o usuário, só se a CPU
anuncia `MD_CLEAR` (`md_clear`) e a detecção marca `mds`; `Switch::Untested` até a medida em metal.
