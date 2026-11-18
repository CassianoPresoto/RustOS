---
id: DEBT-0064
title: Serial precoce confia no que o firmware deixou, só serial e só a da QEMU virt no aarch64
status: Open
date: 2026-11-17
trigger: O primeiro boot em metal, ou o primeiro boot sem serial (só tela)
paid_date:
---

# DEBT 0064 — Serial precoce confia no que o firmware deixou

## O que falta

`early::print` escreve na COM1 (x86_64) ou na PL011 (aarch64) sem programá-las: baud, FIFO e
habilitação são os que o firmware deixou, e na QEMU qualquer valor serve. No aarch64 o endereço é
o da QEMU `virt` ([DEBT-0002](0002-dispositivos-no-endereco-da-qemu-virt.md)), o `TTBR0_EL1` de
`arch/aarch64/src/early.rs` só vale com paginação de 4 níveis (`T0SZ` 16; com 5 níveis a serial
precoce fica desligada) e mapeia um bloco de 2 MiB em `0x0900_0000`, que cobre mais dispositivos da
QEMU `virt` que a PL011.
Sem serial, o pânico precoce não aparece na tela: o renderizador de texto precisa de memória.

## Por que foi aceito

O único alvo é a QEMU, onde a serial funciona sem programação, e o bloco de 2 MiB dura do
início do `kmain` ao `Vm::activate`, sem código que o desreferencie por engano.

## Gatilho

O primeiro boot em metal, ou o primeiro boot sem serial (só tela).

## Como pagar

Programar a UART precoce com o divisor da placa (ler do DTB, DEBT-0002), mapear só a página da PL011 (nível 3, mais uma tabela) e, para metal sem serial, um texto de falha direto no framebuffer que
o Limine entrega, com a fonte de `core/gfx` desenhando sem alocar.
