---
id: DEBT-0015
title: Criptografia do núcleo medida só no host
status: Open
date: 2026-11-17
trigger: O kernel ligar o `Rng` de `core/crypto`, na fase 2
paid_date:
---

# DEBT 0015 — Criptografia do núcleo medida só no host

## O que falta

Os números de ChaCha20, BLAKE2b e do `Rng` no [SDD 07](../sdd/07_desempenho_e_recursos.md)
vêm do host (Apple M5, escalar). O alvo é soft-float sem SIMD, em x86_64 e
aarch64, e o kernel ainda não usa o crate. Também não há o limite de
`bytes_since_seed` que o kernel usa para reabastecer.

## Por que foi aceito

O crate é puro e testado no host; medir no alvo exige a ligação com o kernel e a
semente de hardware, que são da fase 2. Benchmark de latência em TCG não vale
([guidelines 05](../guidelines/05_recursos_e_desempenho.md)).

## Gatilho

O kernel ligar o `Rng`, na fase 2.

## Como pagar

Medir `next_u64` e `fill_bytes` no kernel com aceleração de hardware (HVF ou KVM)
nas duas ISAs, escrever o número no SDD 07 e decidir o limite de reposição.
