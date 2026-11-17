---
id: DEBT-0010
title: Teclado sem comandos ao dispositivo, LEDs nem Num Lock
status: Open
date: 2026-11-17
trigger: O primeiro boot em metal com um teclado PS/2 real
paid_date:
---

# DEBT 0010 — Teclado sem comandos ao dispositivo, LEDs nem Num Lock

## O que falta

O `drivers/i8042` configura só o controlador: nunca escreve na porta de dados
para o teclado. Faltam o reset (`0xFF`), a escolha explícita do conjunto 2 que a
tradução espera (`0xF0`), a taxa de repetição (`0xF3`) e os LEDs (`0xED`). O
`Keyboard` de `core/term` também não tem Num Lock: o teclado numérico sempre
digita números. O autoteste do controlador que falha aborta o `init`, quando o
Linux no x86 segue em frente.

## Por que foi aceito

A QEMU entrega o conjunto 2 de fábrica e não tem LED. Um teclado que sai do
reset já está no conjunto 2; os comandos ao dispositivo só importam em metal.

## Gatilho

O primeiro boot em metal com um teclado PS/2 real.

## Como pagar

Enviar reset e `0xF0 0x02` com espera de `0xFA`, tolerar o autoteste como o
`i8042.c` do Linux no x86, acender LED de Caps Lock quando o `Keyboard` mudar
de estado e acrescentar Num Lock ao estado dele.
