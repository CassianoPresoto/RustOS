---
id: DEBT-0017
title: Dupla liberação de frame no cache por CPU só aparece ao esvaziar o cache
status: Open
date: 2026-11-17
trigger: O primeiro defeito de dupla liberação de frame que o cache esconder, ou o modo de depuração do kernel
paid_date:
---

# DEBT 0017 — Dupla liberação escondida no cache de frames

## O que falta

`FrameCache::deallocate` empilha o frame sem conferir se ele já está no cache: o
buddy só vê a dupla liberação quando o frame sai da pilha (`FrameError::DoubleFree`
vindo de `deallocate` cheio ou de `flush`), longe do chamador que errou. Se o frame
for entregue duas vezes antes disso, dois donos usam o mesmo frame.

## Por que foi aceito

Conferir é varrer a pilha, O(`CAP`), e o caminho quente do cache é O(1) por
requisito (SDD 12). O slab e o buddy detectam dupla liberação na hora.

## Gatilho

O primeiro defeito de dupla liberação de frame que o cache esconder, ou o modo de
depuração do kernel (quando houver), o que vier antes.

## Como pagar

Varredura da pilha em `FrameCache::deallocate` só em build de depuração, ou um bit
por frame de "está num cache" (1 bit por frame, o custo a escrever no SDD 07).
