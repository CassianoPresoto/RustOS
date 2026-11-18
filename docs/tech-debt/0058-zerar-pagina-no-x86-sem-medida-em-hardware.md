---
id: DEBT-0058
title: Zerar página no x86_64 sem medida em hardware, `rep stosb` com ERMS fora
status: Open
date: 2026-11-17
trigger: O primeiro boot do x86_64 com KVM ou em metal
paid_date:
---

# DEBT 0058 — Zerar página no x86_64 sem medida em hardware

## O que falta

No aarch64 o zerar de página usa `DC ZVA` ([SDD 07](../sdd/07_desempenho_e_recursos.md)). No
x86_64, `platform::zero_aligned` é o `write_bytes`, ou seja, o `memset` do alvo, que já é
`rep stosq`. O `rep stosb` com ERMS (`CPUID.7.0:EBX[9]`), que o Linux escolhe
(`clear_page_erms`), não entrou: sob TCG ele custa 8x mais (a QEMU executa um byte por
volta e anuncia ERMS), e não há KVM nem metal x86_64 aqui para medir a CPU de verdade.

## Por que foi aceito

Otimização sem número não entra (guidelines 05). Hoje o `rep stosq` é o do Linux sem ERMS, e
a diferença para `stosb` em hardware moderno é pequena (inferência, sem medida aqui).

## Gatilho

O primeiro boot do x86_64 com KVM ou em metal.

## Como pagar

Medir com a linha `zero bench:` do boot as duas variantes (`stosq` e `stosb`), a de 1 KiB, a
de 4 KiB e a de 4 KiB sobre 1 MiB. Se `stosb` ganhar, decodificar ERMS em
`arch/x86_64/src/features.rs`, escolher a variante em um `zero.rs` com a divisão pura num
módulo à parte (como `zero_plan.rs` do aarch64) e trocar o `zero_aligned` do x86_64.
