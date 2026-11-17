---
id: DEBT-0046
title: Autoteste de SIMD sem os caminhos de XSAVE e sem a metade alta
status: Open
date: 2026-11-17
trigger: A primeira CPU sem `XSAVEOPT`, ou o primeiro boot em metal
paid_date:
---

# DEBT 0046 — Autoteste de SIMD sem os caminhos de XSAVE e sem a metade alta

## O que falta

O autoteste de SIMD roda só o caminho que a CPU de teste escolhe (`XSAVEOPT` no TCG e no
HVF); `MODE_XSAVE` e `MODE_FXSAVE` nunca rodam. Ele escreve e confere só os 16 bytes de
`xmm0`/`v0`: não cobre a metade alta do YMM, o MXCSR nem o FPCR.

## Por que foi aceito

A QEMU `-cpu max` anuncia `XSAVEOPT`, então os outros caminhos precisariam de uma CPU
configurada à mão, e o estado extra pede um padrão de sonda mais largo na `arch/`.

## Gatilho

A primeira CPU sem `XSAVEOPT`, ou o primeiro boot em metal.

## Como pagar

Rodar o autoteste com `-cpu` sem `xsaveopt` e sem `xsave` (um caso por modo); ampliar a
sonda para `ymm0` (metade alta), `MXCSR` e `FPCR` com valores diferentes por tarefa.
