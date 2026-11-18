---
id: DEBT-0070
title: Pilha de 16 KiB sem a medida da falta de página de usuário que resolve
status: Open
date: 2026-11-18
trigger: A falta de página de usuário resolvida dentro do kernel (a paginação sob demanda da 4c, ou a 4f), a primeira que rodar na pilha do kernel de uma thread
paid_date:
---

# DEBT 0070 — Pilha de 16 KiB sem a medida da falta de página de usuário que resolve

## O que falta

A 4a remediu o pico com o programa de usuário embutido: uma syscall (`debug_write`, o caminho mais
fundo da fatia, que formata no console) chega a 2,0 KiB de 16 e a falta fatal de usuário a 1,6 KiB
(medido, `release`, nas duas ISAs; [SDD 15](../sdd/15_tarefas_e_preempcao.md#pico-de-pilha)). Os 16 KiB
ficam. A parte 1 da 4c remediu com handles, canais e espera (`handles`, o pior caminho é o
`channel_write` com cópia de 1 KiB e 512 bytes de handles na pilha): 4,8 KiB de 16 no pico das tarefas
(`stacks:`) e 5,0 KiB (x86_64) e 3,7 KiB (aarch64) de fundo numa syscall (`user: ... kernel stack depth`),
abaixo dos 8 KiB. Falta a falta de página que resolve e volta.

## Por que foi aceito

O pico da 4a é de 12% da pilha, longe dos 50% que pediriam 8 páginas, e subir ou descer sem o
caminho completo trocaria de número duas vezes.

## Gatilho

A falta de página de usuário resolvida dentro do kernel (a paginação sob demanda da 4c, ou a 4f), a primeira que rodar.

## Como pagar

Repetir a linha `stacks:` e a `user: ... kernel stack depth` com a falta resolvida (`make smoke
ARCH=all`, `dev` e `release`). Se o pico passar de 8 KiB, subir para 8 páginas; se a fase fechar
abaixo de 4 KiB, medir 2.
