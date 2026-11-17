---
id: DEBT-0011
title: GIC e timer configurados só para a CPU 0
status: Open
date: 2026-11-17
trigger: O primeiro núcleo secundário subir, na fase 3
paid_date:
---

# DEBT 0011 — GIC e timer configurados só para a CPU 0

## O que falta

No aarch64, `init_interrupts` acorda só o redistribuidor da CPU 0
(`0x080A_0000`). Também só nessa CPU ele liga a PPI do `CNTV` e programa a
interface `ICC_*`. As SPIs são roteadas para a afinidade da CPU que deu boot. No
x86_64, só o Local APIC da CPU que deu boot é ligado, e o timer de um tiro é dele.

## Por que foi aceito

O kernel roda numa CPU só até a fase 3. Não há outro núcleo para receber
interrupção.

## Gatilho

O primeiro núcleo secundário subir, na fase 3.

## Como pagar

Para cada CPU que subir:

- **aarch64:** achar o redistribuidor dela, percorrendo os frames até o
  `GICR_TYPER.Last`, acordá-lo e programar a `ICC_*`.
- **x86_64:** ligar o Local APIC e o timer de um tiro.

Rotear as SPIs pela política de afinidade que a fase 3 definir, e usar SGI/IPI
para acordar outro núcleo.
