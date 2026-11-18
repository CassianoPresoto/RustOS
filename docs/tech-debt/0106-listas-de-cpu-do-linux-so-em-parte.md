---
id: DEBT-0106
title: A detecção de vulnerabilidade usa só parte das listas de CPU do Linux
status: Open
date: 2026-11-18
trigger: O primeiro boot em metal, comparando a linha `mitigations:` com `/sys/devices/system/cpu/vulnerabilities` do Linux na mesma máquina
paid_date:
---

# DEBT 0106 — A detecção de vulnerabilidade usa só parte das listas de CPU do Linux

## O que falta

`arch/x86_64/src/mitigation.rs` leva da `cpu_vuln_whitelist` do Linux v6.12 só o que decide as cinco
vulnerabilidades da linha (família 4 e 5, Atom antigo, Silvermont, Goldmont, AMD, Hygon, Zhaoxin);
a `cpu_vuln_blacklist` e as demais vulnerabilidades ficam fora. **Sem nome na linha:** TAA
(`!TAA_NO` com RTM: a Cascade Lake do dump é afetada pelo Linux), MMIO obsoleto, RFDS, GDS,
Retbleed, SRBDS, L1TF, `swapgs` (DEBT-0082), ITLB multihit e SRSO. No aarch64 as listas de MIDR são as de `cpufeature.c` e `proton-pack.c`, e a CPU fora de todas
conta como afetada pelo BHB (o Linux a conta como não afetada). CPU fora da lista sai vulnerável:
o erro é para o lado do alarme falso.

## Por que foi aceito

A linha só decide `on`/`off (not needed)`/`off (not implemented)` das cinco; uma lista maior não muda
o que o kernel liga hoje.

## Gatilho

O primeiro boot em metal: comparar com o `/sys/devices/system/cpu/vulnerabilities` do Linux na mesma
máquina e tratar cada divergência.

## Como pagar

Acrescentar à tabela as linhas que a comparação mostrar faltando, cada uma com o teste do valor de
CPUID do dump publicado.
