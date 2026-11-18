---
id: DEBT-0056
title: IRQs de dispositivo roteadas só para a CPU de boot
status: Open
date: 2026-11-17
trigger: O primeiro dispositivo com taxa de interrupção que a CPU de boot não aguente, ou o primeiro driver com fila por núcleo (rede, disco)
paid_date:
---

# DEBT 0056 — IRQs de dispositivo roteadas só para a CPU de boot

## O que falta

A serial (SPI 33 no aarch64, pino 4 do I/O APIC no x86_64) e o teclado vão sempre para a CPU 0. A tarefa que espera a entrada acorda em qualquer núcleo por IPI de reescalonar, mas o tratamento da IRQ e o EOI são da CPU de boot.

## Por que foi aceito

A serial e o teclado têm taxa baixa, e o roteamento por afinidade é uma política que precisa de um dispositivo que a justifique.

## Gatilho

O primeiro dispositivo com taxa de interrupção que a CPU de boot não aguente, ou o primeiro driver com fila por núcleo (rede, disco), na fase 6.

## Como pagar

Escolher o destino por dispositivo (`GICD_IROUTER` com `IRM = 1` ou afinidade; destino do I/O APIC por registrador de redirecionamento) e, para MSI/MSI-X, um vetor por CPU.
