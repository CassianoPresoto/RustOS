---
id: DEBT-0014
title: x86_64 sem as salvaguardas que só o metal pede
status: Open
date: 2026-11-17
trigger: O primeiro boot do x86_64 em metal
paid_date:
---

# DEBT 0014 — x86_64 sem as salvaguardas que só o metal pede

## O que falta

- **Calibração do PIT:** desiste depois de `PIT_MAX_POLLS` (50 milhões de leituras
  de porta). Em metal, isso pode ser 50 s ou mais de espera antes do erro.
- **TSC:** o relógio usa o TSC sem conferir a invariância (`CPUID.80000007H`). Numa
  CPU sem TSC invariante, o relógio anda errado quando a frequência muda.
- **Interrupção mascarada por muito tempo:** o `IrqLock` mascara a IRQ durante a
  espera de transmissão da serial e durante o `render` da tela. A 115200 baud, a
  FIFO de 16 bytes da 16550 e o buffer de 1 byte do i8042 podem perder bytes se a
  pessoa digitar enquanto a tela repinta.

## Por que foi aceito

Na QEMU, nada disso aparece. O PIT responde na hora, o TSC do TCG é constante e o
controle de fluxo não perde byte. Os três só se medem em metal.

## Gatilho

O primeiro boot do x86_64 em metal.

## Como pagar

- **PIT:** limite de tempo pelo próprio contador, não por número de leituras.
- **TSC:** conferir o bit de invariância e, sem ele, usar o HPET ou o timer do
  APIC como relógio.
- **Tela e serial:** desenhar e transmitir fora do lock que mascara IRQ, com fila
  própria de saída.
