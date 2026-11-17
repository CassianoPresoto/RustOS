---
id: DEBT-0029
title: Driver de USB HID sem a leitura da especificação
status: Open
date: 2026-11-17
trigger: Antes de escrever o driver de USB HID (fase 8)
paid_date:
---

# DEBT 0029 — Driver de USB HID sem a leitura da especificação

## O que falta

A especificação de USB HID (relatórios, descritores, protocolo de boot) não foi lida. O gamepad da fase 8 e o teclado e mouse USB dependem dela.

## Por que foi aceito

A fase 8 ainda não começou; a pesquisa de jogos só leu o modelo de eventos de gamepad do Linux.

## Gatilho

Antes de escrever o driver de USB HID (fase 8).

## Como pagar

Ler a especificação de HID e a do xHCI, e anotar no SDD o formato dos relatórios e o que vira evento do kernel.
