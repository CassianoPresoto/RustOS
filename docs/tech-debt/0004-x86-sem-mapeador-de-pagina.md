---
id: DEBT-0004
title: x86_64 sem PageMapper; janela de dispositivos só no aarch64
status: Open
date: 2026-11-17
trigger: O primeiro registrador MMIO no x86_64 (Local APIC, fase 1)
paid_date:
---

# DEBT 0004 — x86_64 sem `PageMapper`

## O que falta

Só o `arch/aarch64` implementa `hal::PageMapper`. No x86_64 a janela de
dispositivos e `memory::map_device` existem mas não têm uso, e o kernel as marca
com `allow(dead_code)` nesse alvo.

## Por que foi aceito

O console do x86_64 é a COM1, por porta de E/S: não precisa de mapeamento. Não
havia dispositivo para justificar o mapeador.

## Gatilho

O primeiro registrador MMIO no x86_64 — o Local APIC (`0xFEE0_0000`) ou o HPET,
quando a fase 1 ligar o timer.

## Como pagar

`arch/x86_64` com `descriptor.rs` (bits de PTE: P, RW, PCD, PWT, NX) e uma
caminhada de 4 níveis sobre `CR3`. O `walk.rs` do aarch64 é quase o mesmo
algoritmo: é a hora de decidir se a caminhada genérica vira porta no `hal` com a
política em `core/mm`.
