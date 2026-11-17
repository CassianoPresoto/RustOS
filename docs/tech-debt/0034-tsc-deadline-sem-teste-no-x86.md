---
id: DEBT-0034
title: TSC-deadline sem teste no x86_64
status: Open
date: 2026-11-17
trigger: O primeiro boot do x86_64 com KVM ou em metal
paid_date:
---

# DEBT 0028 — TSC-deadline sem teste no x86_64

## O que falta

O caminho `TimerMode::TscDeadline` (LVT em modo `10b`, `mfence`, `wrmsr` em `IA32_TSC_DEADLINE`) está escrito e o registrador do LVT e a conversão ns→TSC têm teste no host, mas nunca rodou: o TCG da QEMU não anuncia `CPUID.1:ECX[24]`, e o `-cpu max` cai no contador de um tiro do APIC. O jitter de acordar do x86_64 só foi medido com esse.

## Por que foi aceito

Não há máquina x86_64 acelerada aqui (Mac Apple Silicon: o x86_64 só roda em TCG). O caminho de um tiro do APIC cobre o mesmo contrato do kernel (`platform::arm_timer`).

## Gatilho

O primeiro boot do x86_64 com KVM ou em metal (DEBT-0014 e DEBT-0015 pedem o mesmo).

## Como pagar

Subir com `-accel kvm -cpu host`, conferir a linha `sched: ... timer tsc-deadline`, o `bench` e o `timer:` do boot. Se a CPU anunciar TSC-deadline mas o TSC não for invariante, tratar como o Linux (`tsc_deadline` só com TSC confiável). Medir o jitter e pôr no SDD 07.
