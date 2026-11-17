---
id: DEBT-0032
title: virtio-input sem a leitura da especificação
status: Open
date: 2026-11-17
trigger: Antes de escrever o driver de virtio-input, na fase 6
paid_date:
---

# DEBT 0032 — virtio-input sem a leitura da especificação

## O que falta

O roadmap conta com o virtio-input para três coisas: o teclado e o mouse do
aarch64 na QEMU, o marco 1 de jogos no aarch64 e o marco 3 (controle de jogo).
A seção do virtio-input na especificação virtio 1.3 não foi lida. A pesquisa de
jogos o cita só como atalho.

## Por que foi aceito

O driver é da fase 6, depois do PCI. Até lá, a entrada do aarch64 é a serial.

## Gatilho

Antes de escrever o driver de virtio-input, na fase 6.

## Como pagar

- Ler a seção do virtio-input na
  [virtio 1.3](https://docs.oasis-open.org/virtio/virtio/v1.3/virtio-v1.3.html).
- Anotar no SDD o formato do evento (o mesmo do evdev do Linux?) e as filas.
- Só então escrever o adaptador em `drivers/`.
