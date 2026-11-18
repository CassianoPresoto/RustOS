---
id: DEBT-0105
title: A `CSDB` do clamp do Spectre v1 custa 10 ns na syscall nula e o requisito dela não foi lido
status: Open
date: 2026-11-18
trigger: O primeiro boot do aarch64 em metal, ou a leitura do Arm ARM sobre a `CSDB`
paid_date:
---

# DEBT 0105 — A `CSDB` do clamp do Spectre v1 custa 10 ns na syscall nula e o requisito dela não foi lido

## O que falta

A máscara do clamp (`arch/aarch64/src/nospec.rs`) copia a sequência do Linux (`cmp; sbc; csdb`,
`arch/arm64/include/asm/barrier.h`, v6.12). A `CSDB` custa cerca de 10 ns por syscall no HVF
(Apple M5, +21% na chamada nula, [SDD 07](../sdd/07_desempenho_e_recursos.md)), e o Arm ARM não
foi lido para saber se o núcleo precisa dela.

## Por que foi aceito

O Linux usa `CSDB` incondicional no arm64; tirá-la sem ler o requisito seria desligar segurança por
desempenho sem prova ([guidelines 09](../guidelines/09_seguranca.md)).

## Gatilho

O primeiro boot do aarch64 em metal (medir a `CSDB` em outro núcleo), ou a leitura do Arm ARM
sobre a instrução.

## Como pagar

Ler a `CSDB` no Arm ARM e, se o núcleo não especula o valor da máscara, ligá-la só onde precisa
(detecção por MIDR, `FEAT_CSDB` se existir) e medir de novo.
