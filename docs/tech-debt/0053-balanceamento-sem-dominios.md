---
id: DEBT-0053
title: Balanceamento de carga sem domínios, peso nem topologia
status: Open
date: 2026-11-17
trigger: O primeiro boot em máquina com SMT, vários caches ou NUMA, ou a primeira carga em que o número de tarefas por CPU não mede a carga (tarefas de pesos muito diferentes)
paid_date:
---

# DEBT 0053 — Balanceamento de carga sem domínios, peso nem topologia

## O que falta

O pull compara só o número de tarefas prontas e correntes de cada fila (`load`), puxa a de prazo mais distante e trata todas as CPUs como iguais. Não há domínios de escalonamento (SMT, cache, NUMA), peso por nice na carga, custo de migração nem ocupação de cache. O Linux faz isso em `sched-domains` e `sched_balance_rq`.

## Por que foi aceito

Numa máquina simétrica de 4 núcleos e com tarefas do mesmo peso, contar tarefas basta, e foi medido com escala de 3,4x em 4 núcleos ([SDD 07](../sdd/07_desempenho_e_recursos.md)). A topologia só se descobre com ACPI (fase 6).

## Gatilho

O primeiro boot em máquina com SMT, vários caches ou NUMA, ou carga em que contar tarefas erra feio.

## Como pagar

Carga ponderada pelo peso do EEVDF (`sum_weight` da fila), domínios lidos do MADT/SRAT e `pull` com custo de migração; medir com `bench` antes e depois.
