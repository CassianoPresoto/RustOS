---
id: DEBT-0009
title: I/O APIC no endereço fixo e IRQs ISA sem redirecionamento do MADT
status: Open
date: 2026-11-17
trigger: A primeira máquina x86_64 que não seja a QEMU q35, ou o parser de ACPI da fase 6
paid_date:
---

# DEBT 0009 — I/O APIC no endereço fixo e IRQs ISA sem redirecionamento

## O que falta

`arch/x86_64::ioapic::IOAPIC_BASE` vale `0xFEC0_0000` e o kernel roteia os pinos 1
(teclado) e 4 (COM1) como se a IRQ ISA fosse igual ao pino. Em outra máquina o I/O
APIC pode estar em outro endereço, haver mais de um, e a tabela MADT da ACPI pode
redirecionar uma IRQ ISA para outro pino (a QEMU faz isso com a IRQ 0, que vai
para o pino 2, e o kernel não usa). Sem isso o kernel escreve em memória que não é
o I/O APIC, ou a COM1 não interrompe.

## Por que foi aceito

A QEMU `q35` é o único alvo x86_64 hoje (`IO_APIC_DEFAULT_ADDRESS` em
`include/hw/intc/ioapic.h` da QEMU). Ler o MADT exige o parser de ACPI, que só se
paga com a descoberta de dispositivos da fase 6.

## Gatilho

A primeira máquina x86_64 que não seja a QEMU `q35`, ou o parser de ACPI da fase 6,
o que vier primeiro.

## Como pagar

Achar a RSDP pelo pedido do Limine, ler o MADT (entradas tipo 1, I/O APIC, e tipo 2,
*interrupt source override*) e trocar `IOAPIC_BASE` e os números de pino por
consultas à tabela.
