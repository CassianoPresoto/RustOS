---
id: DEBT-0027
title: Driver de virtio-snd sem a leitura da especificação
status: Open
date: 2026-11-17
trigger: Antes de escrever o driver de virtio-snd (fase 6)
paid_date:
---

# DEBT 0027 — Driver de virtio-snd sem a leitura da especificação

## O que falta

A seção 5.14 (dispositivo de som) do virtio 1.3 não foi lida. A pesquisa de jogos citou o virtio-snd como o mais simples dos dispositivos de áudio sem abrir a especificação.

## Por que foi aceito

A pesquisa leu só a documentação do QEMU para o virtio-gpu; áudio é o marco 2 dos jogos e só entra com PCI (fase 6).

## Gatilho

Antes de escrever o driver de virtio-snd (fase 6).

## Como pagar

Ler a seção 5.14 de [virtio 1.3](https://docs.oasis-open.org/virtio/virtio/v1.3/virtio-v1.3.html), anotar filas, formato de PCM e controle no SDD, e só então escrever o adaptador em `drivers/`.
