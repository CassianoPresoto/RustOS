---
id: DEBT-0033
title: Rodízio varre tarefa que não está pronta
status: Open
date: 2026-11-17
trigger: O EEVDF de core/sched entrar no lugar do rodízio, ou o primeiro boot com mais de 16 tarefas
paid_date:
---

# DEBT 0027 — Rodízio varre tarefa que não está pronta

## O que falta

O `RoundRobin` de `core/sched` só tem `admit` e `tick`: não tira ninguém da fila. O kernel deixa os 15 slots de tarefa no anel para sempre e `Scheduler::pick` pula o que está dormindo, bloqueado ou encerrado, até uma volta inteira (15 `tick`s no pior caso, com a ociosa só no fim). Também não há prioridade: quem acorda espera a vez no anel.

## Por que foi aceito

O rodízio é a política de partida, trocada pelo EEVDF por trás do mesmo ponto (`pick`); mexer no crate de `core/sched` estava fora do escopo da fase 3.

## Gatilho

O EEVDF entrar no lugar do rodízio, ou o primeiro boot com mais de 16 tarefas (o anel tem capacidade fixa).

## Como pagar

Com a política de prazo, a fila só tem tarefas prontas: `block`, `wake` e `remove` entram no `core/sched`, o `pick` perde a varredura, e `MAX_TASKS` deixa de ser o tamanho do anel. A preempção por acordar passa a comparar prazos (RUN_TO_PARITY), e o `bench` mede o jitter de acordar de novo.
