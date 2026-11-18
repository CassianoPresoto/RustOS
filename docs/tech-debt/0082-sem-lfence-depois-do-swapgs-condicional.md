---
id: DEBT-0082
title: Sem `lfence` depois do `swapgs` condicional das entradas do x86_64
status: Open
date: 2026-11-18
trigger: O primeiro boot do x86_64 em metal ou com KVM, ou o PR do KPTI (que põe o byte de teste na entrada)
paid_date:
---

# DEBT 0082 — Sem `lfence` depois do `swapgs` condicional das entradas do x86_64

## O que falta

A entrada comum testa o `CS` salvo e só então faz `swapgs`. Sem uma serialização depois do desvio
condicional, uma CPU que especula o caminho errado pode ler o `GS` do usuário como se fosse o do
kernel (a classe do CVE-2019-1125). O Linux põe um `lfence` nos dois ramos. O RustOS não tem.

## Por que foi aceito

A QEMU em TCG não especula. A linha `mitigations:` ainda não tem entrada para esta
vulnerabilidade (a lista `NO_SWAPGS` do Linux não está em `arch/x86_64/src/mitigation.rs`), e o
custo do `lfence` se mede na syscall nula junto com o KPTI.

## Gatilho

O primeiro boot em metal ou com KVM, ou o PR do KPTI, o que vier antes.

## Como pagar

`lfence` depois do `swapgs` e no ramo que não o faz, em `rustos_isr_common`, na entrada paranoica e
no `rustos_debug_entry`; medir o custo na syscall nula. Trazer a `NO_SWAPGS` para a detecção e
acrescentar `swapgs` à linha.
