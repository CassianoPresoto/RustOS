---
id: DEBT-0076
title: Gerador de ASID/PCID sem caminho rápido sem lock
status: Open
date: 2026-11-18
trigger: A medida de troca entre processos da 4e/4f sob HVF/KVM mostrar o lock do gerador
paid_date:
---

# DEBT 0076 — Gerador de ASID/PCID sem caminho rápido sem lock

## O que falta

`AsidAllocator::switch` (`core/mm/src/asid.rs`) é `&mut self`: o kernel o põe atrás de um lock e toda troca de processo, em qualquer CPU, o toma, mesmo quando o espaço já tem valor da geração em curso e nada precisa mudar. Não há desvio sem lock.

## Por que foi aceito

A peça é lógica pura e a ligação (o lock, o `CR3`, o `TTBR0`) é da 4e. Sem medida de troca entre processos com PCID/ASID, não há número que diga que o lock pesa; e um desvio sem lock exige atômicos por CPU (`active`) e uma prova de que a corrida com o esgotamento é segura.

## Gatilho

A medida de troca entre processos da 4e/4f sob HVF/KVM mostrar o lock do gerador (disputa contada ou fração do custo da troca).

## Como pagar

Como o `asid.c` do Linux arm64: `active` por CPU atômico e a geração atômica; se o valor do espaço é da geração em curso e o `active` da CPU não foi zerado pelo esgotamento, troca por `cmpxchg` sem lock; senão cai no `switch` sob lock. O `flush_pending` continua no caminho lento (o esgotamento zera o `active`, o que força a CPU ao lento). Repetir a simulação de TLB com o desvio.
