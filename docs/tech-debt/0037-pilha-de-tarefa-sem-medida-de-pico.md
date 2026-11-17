---
id: DEBT-0037
title: Pilha de tarefa de 32 KiB sem medida de pico
status: Open
date: 2026-11-17
trigger: O primeiro estouro de pilha de uma tarefa, ou o primeiro código de usuário (fase 4)
paid_date:
---

# DEBT 0031 — Pilha de tarefa de 32 KiB sem medida de pico

## O que falta

Cada tarefa nova tem 8 páginas (32 KiB) de pilha, e a ociosa 4 (16 KiB). O tamanho não vem de medida de pico: o handler de IRQ roda na pilha da tarefa interrompida (a ociosa inclusive), e nada mede quanto ela usa. A guarda sem mapeamento garante só que estouro vira relatório de falha, não que o tamanho é o certo.

## Por que foi aceito

O mesmo caso do SDD 13 para a pilha de 64 KiB do boot: é o mínimo conhecido, e sem a medida não há como baixar.

## Gatilho

O primeiro estouro de pilha de uma tarefa (`page fault ... not mapped` na janela de pilhas), ou o primeiro código de usuário (fase 4).

## Como pagar

Preencher a pilha com um padrão no `spawn` e ler a marca d'água no `ps` (uma coluna a mais); medir o pico do shell, do `bench` e do handler de teclado; então escolher o tamanho por tipo de tarefa.
