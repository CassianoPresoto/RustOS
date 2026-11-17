---
id: DEBT-0047
title: `arm_timer`, `next_timer_ns` e a pilha de falha sem teste
status: Open
date: 2026-11-17
trigger: O primeiro boot em metal, ou o primeiro defeito de timer ou de pilha de falha
paid_date:
---

# DEBT 0047 — `arm_timer`, `next_timer_ns` e a pilha de falha sem teste

## O que falta

A saturação do `arm_timer` no x86_64 (contagem de 32 bits do APIC) e o `next_timer_ns`
(o menor entre a fatia e o próximo acordar) não têm teste no host: só o `smoke` os exercita.
No aarch64, o estouro simultâneo da pilha de falha e da de tarefa entra em laço sem
relatório.

## Por que foi aceito

Ambos dependem de `platform` e de `Scheduler` do kernel, que não compilam no host sem
separar a conta pura do acesso ao hardware; o laço do aarch64 é um erro de duas falhas em
cadeia, que o `smoke` não provoca.

## Gatilho

O primeiro boot em metal, ou o primeiro defeito de timer ou de pilha de falha.

## Como pagar

Tirar a conta (`ns` para contagem, saturação, o menor prazo) para uma função pura em
`arch/` ou `core/` com teste no host; no aarch64, um limite de reentrada na pilha de falha
que imprima e pare.
