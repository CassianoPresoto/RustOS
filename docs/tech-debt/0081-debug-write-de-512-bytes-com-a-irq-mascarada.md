---
id: DEBT-0081
title: `debug_write` de até 512 bytes roda com a IRQ mascarada e o lock do console preso
status: Open
date: 2026-11-18
trigger: A 4c ligar a IRQ no despacho de syscall, ou o primeiro boot com UART física
paid_date:
---

# DEBT 0081 — `debug_write` de até 512 bytes roda com a IRQ mascarada e o lock do console preso

## O que falta

A syscall roda mascarada e o `debug_write` escreve até 512 bytes na serial sob o lock do console, que
também mascara a IRQ ([DEBT-0014](0014-x86-sem-salvaguardas-de-metal.md)). Numa UART de 115 200 bps
são 512 × 10 bits ÷ 115 200, cerca de 44 ms sem tique de timer, sem IPI de reescalonar e sem
atender a entrada. Na QEMU a serial é instantânea e a conta não aparece.

## Por que foi aceito

O `debug_write` é provisório (a 4c troca a assinatura) e só o programa embutido do modo de teste o
chama.

## Gatilho

A 4c ligar a IRQ no despacho (a chamada passa a poder ser interrompida), ou o primeiro boot com UART
física, o que vier antes.

## Como pagar

Escrever em pedaços de 64 bytes soltando o lock entre eles, ou reduzir o teto do SDD 17; medir em
UART real.
