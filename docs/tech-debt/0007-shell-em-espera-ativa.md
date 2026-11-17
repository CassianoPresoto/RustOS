---
id: DEBT-0007
title: Shell lê a serial em espera ativa
status: Open
date: 2026-11-17
trigger: A primeira interrupção de UART entregue ao kernel (fase 1)
paid_date:
---

# DEBT 0007 — Shell lê a serial em espera ativa

## O que falta

`kernel/src/system.rs::run_shell` consulta a UART num laço com `spin_loop()`
enquanto ninguém digita. A CPU virtual fica em 100%: na QEMU, cada consulta à
UART sai do guest, e o processo da QEMU ocupa um núcleo do host inteiro.

## Por que foi aceito

O kernel ainda não tem interrupções, e sem elas `hlt`/`wfi` não acordam nunca.
O shell interativo vale ter antes da fase 1; o custo é só de CPU ociosa, e nenhum
número de desempenho do SDD 07 é medido com o shell esperando.

## Gatilho

A primeira interrupção de UART entregue ao kernel (fase 1).

## Como pagar

Ligar a interrupção de recepção da UART (16550 `IER` bit 0; PL011 `IMSC.RXIM`),
pôr os bytes numa fila no handler e trocar o `spin_loop()` por
`Cpu::wait_for_interrupt()` quando a fila estiver vazia.
