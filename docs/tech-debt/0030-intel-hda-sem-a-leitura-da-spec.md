---
id: DEBT-0030
title: Driver de Intel HDA sem a leitura da especificação
status: Open
date: 2026-11-17
trigger: Antes de escrever o driver de Intel HDA, o que só acontece se o primeiro boot de áudio em metal precisar dele
paid_date:
---

# DEBT 0030 — Driver de Intel HDA sem a leitura da especificação

## O que falta

A especificação do Intel HDA (CORB, RIRB, streams) não foi lida. Só vale se o áudio em metal exigir HDA além do virtio-snd.

## Por que foi aceito

O virtio-snd cobre a QEMU; o metal é decisão posterior.

## Gatilho

Antes de escrever o driver de Intel HDA, o que só acontece se o primeiro boot de áudio em metal precisar dele.

## Como pagar

Ler a especificação e anotar no SDD, ou registrar que o HDA foi descartado e dar a dívida por `Dropped`.
