---
id: DEBT-0022
title: Endereços do kernel e do mapa direto impressos no boot, no fault e no pânico
status: Open
date: 2026-11-17
trigger: O primeiro código de usuário (fase 4) ou o primeiro log que saia da máquina
paid_date:
---

# DEBT 0022 — Endereços do kernel no log de boot, no fault e no pânico

## O que falta

O boot imprime `[rustos] kernel base: 0x...` e `vm: direct map at 0x...`, na serial e
na tela. O relatório de `fault` traz o `pc` (`[rustos] fault: ... at pc 0x...`, em
`kernel/src/interrupts.rs`) e o pânico imprime `{info}` com a mensagem inteira, que pode
levar ponteiros (`heap: dealloc de 0x...`). Quem lê o log sabe onde o KASLR pôs a imagem,
o mapa direto e o heap.

## Por que foi aceito

É a medida de que a base muda entre boots, e hoje não há usuário nem rede.

## Gatilho

O primeiro código de usuário (fase 4), ou qualquer destino do log além da serial local.

## Como pagar

Imprimir só em build de depuração, ou tirar o endereço e deixar a linha de
`kernel base` só com o físico. No `fault`, mostrar o `pc` como deslocamento dentro da
imagem; no pânico, tirar o ponteiro da mensagem ou filtrá-lo no `print_panic`.
