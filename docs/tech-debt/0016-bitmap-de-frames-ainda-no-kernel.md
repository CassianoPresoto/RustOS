---
id: DEBT-0016
title: Kernel ainda usa o BitmapFrameAllocator; o buddy e o heap não estão ligados
status: Open
date: 2026-11-17
trigger: O PR que liga o heap do kernel (GlobalAlloc) na fase 2
paid_date:
---

# DEBT 0016 — Kernel ainda usa o BitmapFrameAllocator

## O que falta

`core/mm` tem `BuddyAllocator`, `FrameCache` e `KernelHeap`, testados no host, mas o
`kernel/src/memory.rs` ainda usa o `BitmapFrameAllocator` e não há `GlobalAlloc`.
Os dois alocadores de frames convivem no crate.

## Por que foi aceito

A etapa entregou só a parte pura. Ligar exige o adaptador de `FrameMemory` pelo mapa
direto (`unsafe` no `kernel/`) e os locks por CPU, que são decisão da raiz de
composição.

## Gatilho

O PR que liga o heap do kernel na fase 2. Ele troca o bitmap pelo buddy e apaga o
`BitmapFrameAllocator` (que só serve de base de medição no `bench.rs`, onde pode
ficar como cópia de teste).

## Como pagar

Os quatro passos de "O que o kernel precisa ligar" no
[SDD 12](../sdd/12_memoria_fisica_e_heap.md).
