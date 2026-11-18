---
id: DEBT-0105
title: O clamp do Spectre v1 confere o assembly à mão, e o `CSDB` custa 12 ns na syscall nula
status: Open
date: 2026-11-18
trigger: A troca do `rustc` em `rust-toolchain.toml`, ou o primeiro boot do aarch64 em metal
paid_date:
---

# DEBT 0105 — O clamp do Spectre v1 confere o assembly à mão, e o `CSDB` custa 12 ns na syscall nula

## O que falta

O `array_index_nospec` (`core/hal/src/nospec.rs`) é aritmética pura; que o compilador não a troque por
desvio foi conferido com `objdump` no `rustc` de hoje (`cmov` no x86_64; `sub`, `orr`, `mvn`, `asr`
no aarch64), sem gate. No aarch64 a barreira `CSDB` (`arch/aarch64/src/nospec.rs`) custa cerca de
12 ns por syscall no HVF (Apple M5), e ninguém sabe se o núcleo precisa dela.

## Por que foi aceito

O Linux usa `CSDB` incondicional no arm64; tirá-la sem ler o requisito no Arm ARM seria desligar
segurança por desempenho sem prova ([guidelines 09](../guidelines/09_seguranca.md)).

## Gatilho

A troca da versão do `rustc` (conferir o assembly de novo), ou o primeiro boot do aarch64 em metal
(medir o `CSDB` em outro núcleo).

## Como pagar

Um gate que desmonta `syscall::dispatch` e reprova desvio condicional entre a conta da máscara e o
`AND`; ler o `CSDB` no Arm ARM e, se o núcleo não especula o valor, ligá-lo só onde precisa.
