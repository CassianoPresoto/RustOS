---
id: DEBT-0002
title: Endereços da PL011 e do GICv3 fixos nos da QEMU virt
status: Open
date: 2026-11-17
trigger: A primeira placa aarch64 que não seja a QEMU virt
paid_date:
---

# DEBT 0002 — Endereços da PL011 e do GICv3 fixos nos da QEMU `virt`

## O que falta

`arch/aarch64` exporta `QEMU_VIRT_PL011` (`0x0900_0000`), `QEMU_VIRT_GICD`
(`0x0800_0000`) e `QEMU_VIRT_GICR` (`0x080A_0000`), e `intid.rs` o INTID da UART
(SPI 33) e o do timer virtual (PPI 27). O `kernel/src/platform/aarch64.rs` mapeia
esses endereços e programa esses números. Em outra placa os dispositivos estão em
outro lugar, ou são outros: o kernel escreveria em memória que não é a deles, sem
saída nenhuma. O redistribuidor mapeado é só o da CPU 0 (128 KiB).

## Por que foi aceito

A QEMU `virt` é o único alvo aarch64 hoje. Ler os endereços do device tree exige um
parser de FDT, que só se paga quando houver uma segunda placa.

## Gatilho

A primeira placa aarch64 que não seja a QEMU `virt` (Raspberry Pi, servidor ARM,
outra máquina da QEMU).

## Como pagar

Pedir o DTB ao Limine (`LIMINE_DTB_REQUEST`), escrever um leitor de FDT em `core/`
(puro, testável) e procurar o nó `arm,pl011` do `/chosen/stdout-path`, o
`arm,gic-v3` (`reg`: distribuidor e redistribuidores) e o `arm,armv8-timer`
(`interrupts`). Em servidor ARM com ACPI, a SPCR e a MADT.
