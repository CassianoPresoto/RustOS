---
id: DEBT-0071
title: Shell usa dois terços da pilha do boot em release
status: Open
date: 2026-11-18
trigger: O `ps` em `release` mostrar o pico do shell acima de 48 KiB dos 64, ou a 4a pôr o shell numa pilha de thread de 16 KiB
paid_date:
---

# DEBT 0071 — Shell usa dois terços da pilha do boot em release

## O que falta

O shell roda na pilha de 64 KiB do boot e, em `release`, chega a 38 KiB no primeiro `ps` e a 42,6 KiB depois do `bench` (aarch64 e x86_64); em `dev` são 17 e 23 KiB. A causa não foi investigada: é provável que a função que despacha os comandos inlinar todos eles num quadro só.

## Por que foi aceito

Cabe na pilha de 64 KiB (margem de 1,5x), o `ps` mostra o pico, e o shell é a única thread nessa pilha.

## Gatilho

O `ps` em `release` mostrar o shell acima de 48 KiB, ou a fase 4 mover o shell para uma tarefa comum (pilha de 16 KiB, que ele não cabe).

## Como pagar

Achar o quadro grande (`-Z emit-stack-sizes` pede nightly; sem ele, medir com a marca d'água antes e depois de tirar `#[inline]` de cada comando) e partir o despacho; ou pôr o `bench` e o `ps` numa tarefa própria.
