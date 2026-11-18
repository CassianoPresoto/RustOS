---
id: DEBT-0060
title: Recuo de STP do zerar página sem cobertura automática
status: Open
date: 2026-11-17
trigger: A primeira CPU aarch64 com DCZID_EL0.DZP ligado, ou o primeiro modelo da QEMU que o ligue
paid_date:
---

# DEBT 0060 — Recuo de `STP` do zerar página sem cobertura automática

## O que falta

`arch/aarch64/src/zero.rs` cai num laço de `STP` quando `DCZID_EL0.DZP` proíbe o `DC ZVA`.
O `make smoke` nunca passa por ele: HVF e TCG (`max`, `a64fx`, `cortex-a72`) deixam `DZP`
limpo. O laço foi rodado à mão, forçando o recuo no código, com o autoteste do heap verde
([SDD 13](../sdd/13_memoria_virtual.md)). As pontas `head` e `tail` da faixa já têm
cobertura automática: o `make pr` e o CI rodam o smoke do aarch64 com
`RUSTOS_ACCEL=tcg RUSTOS_CPU=a64fx` (bloco de 256 bytes). Falta só o laço de `STP`.

## Por que foi aceito

Não há como ligar `DZP` sem mudar o código, e o laço é de 4 `STP` por volta, testado à mão.

## Gatilho

A primeira CPU aarch64 com `DZP` ligado, ou o primeiro modelo da QEMU que o ligue.

## Como pagar

Rodar o `make smoke` com esse modelo no `make pr` e no CI, ou expor um parâmetro de boot
que force o recuo e rodar o autoteste do heap com ele.
