---
id: DEBT-0102
title: Spectre v4 sem mitigação (SSBD no x86_64, SSBS no aarch64)
status: Open
date: 2026-11-18
trigger: O primeiro boot em metal
paid_date:
---

# DEBT 0102 — Spectre v4 sem mitigação (SSBD no x86_64, SSBS no aarch64)

## O que falta

A linha `mitigations:` diz `ssb off (not implemented)` onde a CPU tem Spectre v4: no x86_64, a que
não anuncia `SSB_NO` nem está na lista do Linux; no aarch64, toda fora da lista de CPUs imunes
(`SSBS` existe em várias e o kernel não o liga). O padrão do Linux é por processo, sob pedido
(`prctl`), e o RustOS não tem a interface.

## Por que foi aceito

A interface por processo é contrato de fora (caso 6 do `AGENTS.md`) e a QEMU não modela o efeito.

## Gatilho

O primeiro boot em metal.

## Como pagar

`SPEC_CTRL.SSBD` (x86_64, bit 2 do MSR `0x48`) ou `PSTATE.SSBS` (aarch64) na troca de contexto de
processo marcado; decidir a política (padrão desligado, como o Linux) e a interface no SDD 17.
