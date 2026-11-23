---
id: DEBT-0157
title: Ramo de ASID de 16 bits (TCR_EL1.AS) sem execução
status: Open
date: 2026-11-23
trigger: O primeiro boot aarch64 com `ASIDBits = 16` (linha `tlb tags: asid 16-bit`)
paid_date:
---

# DEBT 0157 — Ramo de ASID de 16 bits (TCR_EL1.AS) sem execução

## O que falta

O ramo de `TCR_EL1.AS` (bit 36) para ASID de 16 bits, em `arch/aarch64/src/user.rs` (`init_user` com `wide_asid`),
nunca rodou. O aarch64 da QEMU usada aqui (TCG e HVF) anuncia `ID_AA64MMFR0_EL1.ASIDBits = 8`, então a linha sai
`asid 8-bit` e o gerador usa 8 bits. Com 16 bits o gerador passaria a ter 65 535 etiquetas e o `TCR_EL1.AS` mudaria.

## Por que foi aceito

Nenhuma máquina ao alcance anuncia 16 bits. O caminho de 8 bits foi exercitado (HVF e TCG), e o de 16 bits tem o mesmo
código de troca, com a largura do gerador lida do mesmo registrador.

## Gatilho

O primeiro boot aarch64 em que a linha `tlb tags:` saia `asid 16-bit`.

## Como pagar

Rodar `make smoke ARCH=aarch64`, conferir o `asid self-test: ok` e a medida `asid bench`, e confirmar que
`TCR_EL1.AS` foi escrito (um `mrs` no autoteste, se faltar evidência). Registrar os números no SDD 07.
