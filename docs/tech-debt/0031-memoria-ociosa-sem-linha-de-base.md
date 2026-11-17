---
id: DEBT-0031
title: Alvo de memória ociosa sem linha de base medida
status: Open
date: 2026-11-17
trigger: Antes de fechar a fase 5
paid_date:
---

# DEBT 0031 — Alvo de memória ociosa sem linha de base medida

## O que falta

A meta de memória ociosa não tem número: a pesquisa não achou medida confiável de Linux mínimo nem de Windows 11. Só existem a memória livre do kernel depois do boot (SDD 07) e números de blog de outras distribuições.

## Por que foi aceito

Um alvo sem medida seria chute, e o guidelines 05 não deixa otimização sem número. A comparação só faz sentido com espaço de usuário.

## Gatilho

Antes de fechar a fase 5.

## Como pagar

Seguir o método do [SDD 08](../sdd/08_roadmap.md#memória-ociosa-o-alvo-vem-de-uma-medida): Linux mínimo na mesma QEMU e mesma RAM, três boots frios, mediana de `MemAvailable`; depois o RustOS; fixar o alvo no SDD 07.
