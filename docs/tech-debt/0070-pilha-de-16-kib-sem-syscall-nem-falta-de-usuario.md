---
id: DEBT-0070
title: Pilha de 16 KiB remedida só com syscall e falta fatal de usuário, sem handles, canais nem falta resolvida
status: Open
date: 2026-11-18
trigger: A 4c (handles, canais, VMAR) ou a 4f (falta de página que zera) rodarem na pilha do kernel de uma thread
paid_date:
---

# DEBT 0070 — Pilha de 16 KiB remedida só com syscall e falta fatal de usuário, sem handles, canais nem falta resolvida

## O que falta

A 4a remediu o pico com o programa de usuário embutido: uma syscall (`debug_write`, o caminho mais
fundo da fatia, que formata no console) chega a 2,0 KiB de 16 e a falta fatal de usuário a 1,6 KiB
(medido, `release`, nas duas ISAs; [SDD 15](../sdd/15_tarefas_e_preempcao.md#pico-de-pilha)). Os 16 KiB
ficam. Falta o que a fase ainda não tem: handles, canais, a árvore de regiões e a falta de página
que resolve e volta.

## Por que foi aceito

O pico da 4a é de 12% da pilha, longe dos 50% que pediriam 8 páginas, e subir ou descer sem o
caminho completo trocaria de número duas vezes.

## Gatilho

A 4c ou a 4f, a que primeiro rodar handles, canais ou a falta resolvida na pilha do kernel.

## Como pagar

Repetir a linha `stacks:` e a `user: ... kernel stack depth` com os programas da fatia (`make smoke
ARCH=all`, `dev` e `release`). Se o pico passar de 8 KiB, subir para 8 páginas; se a fase fechar
abaixo de 4 KiB, medir 2.
