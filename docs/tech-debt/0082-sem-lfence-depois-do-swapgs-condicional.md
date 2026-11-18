---
id: DEBT-0082
title: Sem `lfence` depois do `swapgs` condicional das entradas do x86_64
status: Open
date: 2026-11-18
trigger: O primeiro boot do x86_64 em metal ou com KVM, ou a 4e (mitigações de CPU)
paid_date:
---

# DEBT 0082 — Sem `lfence` depois do `swapgs` condicional das entradas do x86_64

## O que falta

A entrada comum testa o `CS` salvo e só então faz `swapgs`. Sem uma serialização depois do desvio
condicional, uma CPU que especula o caminho errado pode ler o `GS` do usuário como se fosse o do
kernel (a classe do CVE-2019-1125). O Linux põe um `lfence` nos dois ramos. O RustOS não tem.

## Por que foi aceito

A QEMU em TCG não especula, e as mitigações de CPU são da 4e, com custo medido
([SDD 17](../sdd/17_processos_isolamento_e_syscalls.md#mitigações-de-cpu)).

## Gatilho

O primeiro boot em metal ou com KVM, ou a 4e.

## Como pagar

`lfence` depois do `swapgs` e no ramo que não o faz, em `rustos_isr_common`, na entrada paranoica e
no `rustos_debug_entry`; medir o custo na syscall nula.
