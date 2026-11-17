---
id: DEBT-0041
title: Escalonador do x86_64 sem medida fora do TCG
status: Open
date: 2026-11-17
trigger: O primeiro boot do x86_64 com KVM ou em metal
paid_date:
---

# DEBT 0041 — Escalonador do x86_64 sem medida fora do TCG

## O que falta

No x86_64 o `pick_next` do EEVDF custa de 670 a 730 ns e a troca de contexto de 1,1 a
1,2 µs, sob TCG (`bench`). O TCG interpreta o `i128` do EEVDF (`__divti3`) e a divisão
de 128 bits custa muito mais ali do que custaria a instrução `div` de hardware. O número
não diz nada sobre uma CPU x86_64 de verdade, e a política não foi medida em nenhuma.

## Por que foi aceito

Só há QEMU sem KVM neste host (macOS em Apple Silicon): o x86_64 roda sempre em TCG
(guidelines 05). O aarch64, com HVF, dá a medida de hardware da mesma política.

## Gatilho

O primeiro boot do x86_64 com KVM ou em metal (o mesmo de DEBT-0015 e DEBT-0034).

## Como pagar

Rodar `make screenshot ARCH=x86_64 PROFILE=release CMDS="bench;bench;bench"` com KVM e
escrever o número no SDD 07. Se o `pick_next` passar de 100 ns, trocar o `i128` por
`u64` com a janela de lag limitada a duas fatias em `core/sched`.
