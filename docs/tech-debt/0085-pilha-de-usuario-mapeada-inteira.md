---
id: DEBT-0085
title: Pilha inicial de usuário mapeada inteira na criação
status: Open
date: 2026-11-18
trigger: A falta de página de usuário resolvida dentro do kernel entrar (a 4c ou a 4f), ou a medida do menor processo na 4f
paid_date:
---

# DEBT 0085 — Pilha inicial de usuário mapeada inteira na criação

## O que falta

O carregador mapeia as 16 páginas (64 KiB) da pilha inicial na criação, cada uma num frame zerado. Das 32
páginas que o menor processo (o `init`) consome, 16 são só a pilha, da qual ele usa uma. O resto
([SDD 07](../sdd/07_desempenho_e_recursos.md)) é imagem (4), tabelas (7) e o que o kernel guarda por tarefa (5).

## Por que foi aceito

Sem falta de página resolvida no kernel, toda página que o programa pode tocar precisa existir antes de ele
rodar. A falta que cria a página no primeiro toque é da 4f.

## Gatilho

A falta de página de usuário resolvida dentro do kernel entrar, ou a medida do menor processo na 4f.

## Como pagar

Mapear só a última página da pilha (a dos argumentos) e criar as outras no primeiro toque, com
`Vmar::check_fault` decidindo (a região `Kind::Stack` já está na árvore, com as guardas).
