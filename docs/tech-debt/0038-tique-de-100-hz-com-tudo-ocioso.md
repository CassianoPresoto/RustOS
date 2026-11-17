---
id: DEBT-0038
title: Tique de 100 Hz com tudo ocioso
status: Open
date: 2026-11-17
trigger: A primeira medida de energia ou de uso de CPU ocioso em metal, ou em placa a bateria
paid_date:
---

# DEBT 0032 — Tique de 100 Hz com tudo ocioso

## O que falta

O timer é de um tiro e o próximo disparo é o menor entre o próximo tique e o próximo acordar, mas o tique de 100 Hz é sempre um dos dois: a CPU acorda a cada 10 ms mesmo com a ociosa em `hlt`/`wfi` e ninguém dormindo.

## Por que foi aceito

O tique é também a fatia do rodízio e o relógio do autoteste de timer; o NO_HZ_IDLE do Linux exige parar o tique ao entrar na ociosa e recuperar o tempo ao sair.

## Gatilho

A primeira medida de energia ou de uso de CPU ocioso em metal, ou em placa a bateria.

## Como pagar

Na ociosa, armar só o próximo acordar (ou nada) e, ao sair dela, contar os tiques perdidos a partir do relógio; ver `docs.kernel.org/timers/no_hz.html` (NO_HZ_IDLE).
