---
id: DEBT-0015
title: Criptografia do núcleo sem medida do x86_64 com aceleração
status: Open
date: 2026-11-17
trigger: O primeiro boot do x86_64 com KVM ou em metal
paid_date:
---

# DEBT 0015 — Criptografia do núcleo sem medida do x86_64 com aceleração

## O que falta

O kernel mede o ChaCha20 e a reposição do gerador no boot (linha `random bench:`,
[SDD 07](../sdd/07_desempenho_e_recursos.md)). No aarch64 o número vale: HVF. No
x86_64 só há TCG neste ambiente (host aarch64), e número de TCG não serve para
comparar com outro sistema ([guidelines 05](../guidelines/05_recursos_e_desempenho.md)).

## Por que foi aceito

Não há máquina x86_64 com KVM ou metal à mão. A medida do aarch64 já fixou o limite
de reposição (`RESEED_BYTES`), que não depende da ISA.

## Gatilho

O primeiro boot do x86_64 com KVM ou em metal.

## Como pagar

Rodar `make smoke ARCH=x86_64` com KVM, copiar a linha `random bench:` para o SDD 07
e conferir que a reposição continua abaixo de 1% do tempo de gerar `RESEED_BYTES`.
Então apagar este arquivo.
