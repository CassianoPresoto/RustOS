---
id: DEBT-0024
title: Kernel aarch64 relocável só com `-z notext` (TEXTREL)
status: Open
date: 2026-11-17
trigger: A `core` pré-compilada do alvo aarch64 passar a vir com relocação PIC, ou o primeiro build com build-std
paid_date:
---

# DEBT 0024 — aarch64 PIE com TEXTREL

## O que falta

A `core` e a `alloc` pré-compiladas de `aarch64-unknown-none-softfloat` são estáticas:
têm `R_AARCH64_ABS64` em `.rodata`. O link do PIE só passa com `-z notext`, e o ELF
sai com `DT_FLAGS: TEXTREL`. O x86_64 não precisa disso.

## Por que foi aceito

O Limine aplica as relocações antes de o kernel mapear `.rodata` como leitura, e o
kernel sobe e sorteia a base nas duas ISAs. A alternativa é nightly com build-std.

## Gatilho

O alvo aarch64 passar a distribuir `core` com relocação PIC, ou o primeiro build
com `build-std`.

## Como pagar

Tirar `-znotext` de `kernel/build.rs` quando o link passar sem ele.
