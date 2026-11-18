---
id: DEBT-0104
title: Injeção de histórico de desvio sem mitigação (BHI no x86_64, Spectre-BHB no aarch64)
status: Open
date: 2026-11-18
trigger: O primeiro boot em metal
paid_date:
---

# DEBT 0104 — Injeção de histórico de desvio sem mitigação (BHI no x86_64, Spectre-BHB no aarch64)

## O que falta

A linha `mitigations:` diz `bhb off (not implemented)` onde a CPU tem a vulnerabilidade: no x86_64,
a com eIBRS (ou em máquina virtual) sem `BHI_NO`; no aarch64, a sem `CSV2 = 3` e sem `ECBHB`, que
esteja nas listas do Linux ou em nenhuma. O kernel não liga `BHI_DIS_S`, não faz o laço de desvios
na entrada nem usa `CLRBHB`.

## Por que foi aceito

Nada disso se exercita fora de hardware, e a regra do aarch64 para CPU fora das listas é mais
conservadora que a do Linux ([SDD 17](../sdd/17_processos_isolamento_e_syscalls.md#mitigações-de-cpu)).

## Gatilho

O primeiro boot em metal.

## Como pagar

x86_64: `SPEC_CTRL.BHI_DIS_S` (bit 10) onde `BHI_CTRL` existe, senão o laço de `clear_bhb`. aarch64:
`ECBHB` não pede nada; `CLRBHB` e o laço por MIDR (`k` = 8, 11, 24, 32) nos vetores de entrada.
