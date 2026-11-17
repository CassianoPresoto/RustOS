---
id: DEBT-0002
title: Endereço da PL011 fixo no da QEMU virt
status: Open
date: 2026-11-17
trigger: A primeira placa aarch64 que não seja a QEMU virt
paid_date:
---

# DEBT 0002 — Endereço da PL011 fixo no da QEMU `virt`

## O que falta

`arch/aarch64` exporta `QEMU_VIRT_PL011 = 0x0900_0000` e o
`kernel/src/platform/aarch64.rs` mapeia a serial nesse endereço. Em outra placa a
serial está em outro lugar (ou é outro dispositivo), e o kernel escreveria em
memória que não é a UART — sem saída nenhuma.

## Por que foi aceito

A QEMU `virt` é o único alvo aarch64 hoje. Ler o endereço do device tree exige um
parser de FDT, que só se paga quando houver uma segunda placa.

## Gatilho

A primeira placa aarch64 que não seja a QEMU `virt` (Raspberry Pi, servidor ARM,
outra máquina da QEMU).

## Como pagar

Pedir o DTB ao Limine (`LIMINE_DTB_REQUEST`), escrever um leitor de FDT em `core/`
(puro, testável) e procurar o nó `arm,pl011` do `/chosen/stdout-path`. Em servidor
ARM com ACPI, a tabela SPCR.
