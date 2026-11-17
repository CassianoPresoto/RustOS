---
id: DEBT-0029
title: Driver de virtio-gpu 2D sem a leitura da especificação
status: Open
date: 2026-11-17
trigger: Antes de escrever o driver de virtio-gpu 2D (fase 11)
paid_date:
---

# DEBT 0029 — Driver de virtio-gpu 2D sem a leitura da especificação

## O que falta

A parte 2D do virtio-gpu (recursos, transferência, `flush`, page flip e vsync) não foi lida na especificação. A pesquisa leu a documentação do QEMU, que descreve virgl, Venus e gfxstream, não o 2D.

## Por que foi aceito

Os marcos 4 e 5 dos jogos só entram na fase 11.

## Gatilho

Antes de escrever o driver de virtio-gpu 2D (fase 11).

## Como pagar

Ler a seção do virtio-gpu em [virtio 1.3](https://docs.oasis-open.org/virtio/virtio/v1.3/virtio-v1.3.html), em especial o que o 2D oferece para page flip e vsync, e anotar no SDD.
