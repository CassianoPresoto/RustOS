---
id: DEBT-0054
title: Heap do kernel com um lock global
status: Open
date: 2026-11-17
trigger: O primeiro programa que aloque do heap em todos os núcleos ao mesmo tempo (fase 4), ou a disputa do heap passar de 10% do tempo de uma CPU no `bench`
paid_date:
---

# DEBT 0054 — Heap do kernel com um lock global

## O que falta

`HEAP` é um `IrqLock` só. Com 4 núcleos alocando sem parar, o `bench` mede disputa pelo lock do heap (milhares de tentativas que o acharam preso), enquanto o frame solto, com o cache por CPU, não disputa nada. O slab de cada classe é de todos.

## Por que foi aceito

Nenhuma tarefa do kernel aloca em laço em vários núcleos; o custo medido é de dezenas de ns por alocação ([SDD 07](../sdd/07_desempenho_e_recursos.md)).

## Gatilho

O primeiro programa que aloque do heap em todos os núcleos ao mesmo tempo (fase 4), ou a disputa do heap passar de 10% do tempo de uma CPU no `bench`.

## Como pagar

Um slab por CPU por classe de tamanho, com devolução cruzada por lista atômica (o `kmem_cache` do Linux), mantendo `KernelHeap` em `core/mm`; medir a tempestade de alocação do `bench` antes e depois.
