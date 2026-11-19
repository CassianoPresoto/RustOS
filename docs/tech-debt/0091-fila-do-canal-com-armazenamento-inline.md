---
id: DEBT-0091
title: Fila do canal com armazenamento inline, 7,2 KiB por par de pontas
status: Open
date: 2026-11-18
trigger: A medida de memória da 4f ou do SDD 07 mostrar canais entre as maiores parcelas do kernel, ou um processo precisar de mais de 100 canais vivos
paid_date:
---

# DEBT 0091 — Fila do canal com armazenamento inline, 7,2 KiB por par de pontas

## O que falta

`Channel<T>` guarda 16 mensagens por ponta num anel inline, cada uma com 8 handles em trânsito:
`Message` tem 224 bytes e o par 7 256 bytes (medido com `size_of`, host, `rustc` 1.100.0), ocupados
ou não. O núcleo não aloca, então a fila não cresce por demanda.

O kernel o paga de outro jeito também: o par não cabe na pilha de 16 KiB da tarefa (construí-lo
estourava a pilha e o `init` morria em `#DF`), então nasce por cópia de um modelo vazio de 7 KiB que mora
na imagem (`kernel/src/object.rs`, `EMPTY_PAIR`; `rodata` do kernel +9 KiB na parte 1 da 4c) e leva
dois `unsafe` com `// SAFETY:`. Medido pelo revisor: sem ele a imagem seria **6,8 KiB menor no x86_64 e 6,6 KiB no aarch64**.

## Por que foi aceito

`core/` não chama o alocador (guidelines 05). Com 256 pontas por processo no teto, o pior caso é de
cerca de 0,9 MiB por processo; o uso típico (o canal de eventos e o de boot) é de dezenas de KiB.

## Gatilho

A medida de memória da 4f mostrar canais entre as maiores parcelas, ou um processo com mais de 100
canais vivos.

## Como pagar

Guardar a mensagem inteira numa caixa que o kernel entrega (`Channel` genérico sobre o corpo da
mensagem) e a fila como anel de ponteiros, ou um anel que cresce pelo `SlotStorage`. Com a fila fora
do par, o modelo de 7 KiB e os `unsafe` saem.
