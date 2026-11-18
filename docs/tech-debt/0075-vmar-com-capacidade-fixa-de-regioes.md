---
id: DEBT-0075
title: VMAR com capacidade fixa de regiões e deslocamento O(N)
status: Open
date: 2026-11-18
trigger: O primeiro processo que passar da capacidade escolhida pela 4b, ou a medida de `reserve`/`unmap` aparecer no perfil de criação de processo da 4f
paid_date:
---

# DEBT 0075 — VMAR com capacidade fixa de regiões e deslocamento O(N)

## O que falta

`Vmar` (`core/mm/src/vmar.rs`) guarda as regiões num vetor ordenado de capacidade fixa, dada pelo chamador. Cheio é `VmarError::Full`; não cresce. `reserve` e `unmap` deslocam até N regiões de 32 bytes: no host, o par `reserve` + `unmap` no meio custa 36 ns com 16 regiões, 120 ns com 256 e **1,1 µs com 4 096** ([SDD 07](../sdd/07_desempenho_e_recursos.md#árvore-de-regiões-e-gerador-de-asid-fase-4-medido-no-host)).

## Por que foi aceito

A 4b fixou 32 regiões por processo (`REGION_CAPACITY` em `kernel/src/loader.rs`, 1 KiB): no máximo 16 `PT_LOAD`, a pilha e folga para os mapeamentos da 4c. O `init` e os filhos da fase 4 têm dezenas de regiões (inferência, não medida em processo real). O vetor ordenado acha em O(log N), custa 32 bytes por região e não aloca sozinho; uma árvore balanceada pesaria em código e memória por nó sem caso que a peça.

## Gatilho

O primeiro processo que passar da capacidade escolhida pela 4b, ou a medida de `reserve`/`unmap` aparecer no perfil de criação de processo da 4f.

## Como pagar

Trocar o vetor por uma árvore (ou vetor em blocos) e deixar o armazenamento crescer por demanda, atrás da mesma API (`reserve`, `allocate`, `unmap`, `find`, `check_fault`). Os testes contra o modelo ingênuo e o `bench_vmar` valem como estão.
