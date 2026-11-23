---
id: DEBT-0122
title: Shootdown de TLB do x86_64 sem medida em KVM
status: Open
date: 2026-11-18
trigger: O primeiro boot do x86_64 com KVM ou em metal
paid_date:
---

# DEBT 0122 — Shootdown de TLB do x86_64 sem medida em KVM e uma invalidação por página

## O que falta

O IPI de invalidação do x86_64 vai de CPU em CPU com uma caixa por par (origem, destino) e espera todas as confirmações; o número que existe (8 µs por página no TCG) é de TCG e não vale como latência. O `vmar_unmap` já agrupa 32 páginas por IPI (`tlb::shoot_range`); o corte de 32 do `flush_pages` não tem medida. As caixas dividem linhas de cache ([DEBT-0061](0061-percpu-sem-separar-linhas-de-cache.md)).

## Por que foi aceito

A fase 4 só invalida uma página por vez (a cópia da página zero, a remoção de uma folha) e o TCG não mede o custo do IPI.

## Gatilho

O primeiro boot do x86_64 com KVM ou em metal.

## Como pagar

Medir o IPI de ida e volta e o `invlpg` remoto sob KVM e em metal; alinhar cada caixa à linha de cache; decidir o corte de `flush_pages` pelo número medido.
