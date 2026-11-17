---
id: DEBT-0022
title: Endereço-base do kernel e do mapa direto impressos no boot
status: Open
date: 2026-11-17
trigger: O primeiro código de usuário (fase 4) ou o primeiro log que saia da máquina
paid_date:
---

# DEBT 0022 — Endereço-base do kernel no log de boot

## O que falta

O boot imprime `[rustos] kernel base: 0x...` e `vm: direct map at 0x...`, na serial e
na tela. Quem lê o log sabe onde o KASLR pôs a imagem e o mapa direto.

## Por que foi aceito

É a medida de que a base muda entre boots, e hoje não há usuário nem rede.

## Gatilho

O primeiro código de usuário (fase 4), ou qualquer destino do log além da serial local.

## Como pagar

Imprimir só em build de depuração, ou tirar o endereço e deixar a linha de
`kernel base` só com o físico.
