---
id: DEBT-0067
title: Log do boot normal sem medida em UART física
status: Open
date: 2026-11-18
trigger: O primeiro boot em metal, ou em qualquer máquina com UART física lenta
paid_date:
---

# DEBT 0067 — Log do boot normal sem medida em UART física

## O que falta

O boot normal imprime 11 a 12 linhas (cerca de 700 bytes) e o de teste, 30. Na serial da QEMU uma linha custa de
0,3 a 0,4 ms; numa UART de 115 200 baud seria cerca de 7 ms por linha de 80 bytes, ou 80 ms só de log. O
número de linhas do modo normal foi escolhido pelo que os gates conferem, não por uma medida em UART real.

## Por que foi aceito

Não há boot em metal ainda ([DEBT-0001](0001-boot-pelo-limine.md), [DEBT-0064](0064-serial-precoce-so-confia-no-firmware.md)); o corte
de linhas já tira cerca de 40% dos bytes do boot normal sem perder o que o `smoke-fast` exige.

## Gatilho

O primeiro boot em metal, ou em qualquer máquina com UART física, em que o `boot time:` do modo
normal passe de 50 ms.

## Como pagar

Medir o `boot time:` lá. Se a serial dominar, imprimir no modo normal só `boot time:` e `boot ok` (e o que falhar), e deixar
`kaslr:`, `protections:`, `reclaimed:` e `cpus:` para o `fetch`; o `smoke-fast` passa a exigi-los pelo `fetch`, ou pelo modo de teste.
