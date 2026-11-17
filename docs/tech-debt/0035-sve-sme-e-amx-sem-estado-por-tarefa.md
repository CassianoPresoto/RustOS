---
id: DEBT-0035
title: SVE, SME e AMX sem estado por tarefa
status: Open
date: 2026-11-17
trigger: O primeiro código de usuário (fase 4), ou a primeira CPU de teste com SVE, SME ou AMX
paid_date:
---

# DEBT 0035 — SVE, SME e AMX sem estado por tarefa

## O que falta

A troca de contexto salva V0–V31, FPSR e FPCR no aarch64, e x87, SSE, AVX e AVX-512 (se houver) no x86_64. `CPACR_EL1.ZEN` e `SMEN` seguem zero (SVE e SME trapeiam), e `XCR0` não liga AMX (bits 17 e 18) nem PKRU.

## Por que foi aceito

O kernel não usa nenhum deles, e as áreas teriam de ser alocadas por tarefa e por vetor (SVE tem VL configurável; AMX tem 8 KiB de tile, e o Linux exige permissão por processo).

## Gatilho

O primeiro código de usuário (fase 4): programa que usa SVE, SME ou AMX.

## Como pagar

aarch64: ligar `ZEN`/`SMEN` e salvar por trap de primeiro uso, como `do_sve_acc` do Linux (`fpsimd.c`), com o VL padrão de 64 bytes. x86_64: `XCR0` com AMX só atrás de `arch_prctl(ARCH_REQ_XCOMP_PERM)` equivalente; a área cresce por tarefa (ver a pesquisa `fpu-simd.md`).
