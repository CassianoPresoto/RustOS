---
id: DEBT-0112
title: `TERMINATED` do processo sem teste de espera bloqueante
status: Open
date: 2026-11-18
trigger: O `process_create` da parte 3 da 4c (o primeiro processo que espera outro)
paid_date:
---

# DEBT 0112 — `TERMINATED` do processo sem teste de espera bloqueante

## O que falta

O objeto `Process` levanta `TERMINATED` e acorda quem espera (`ProcessObj::terminate`), mas na parte 1 só o canal tem teste de espera que bloqueia e acorda (`handles`). O `init` só confere que o próprio processo **não** está terminado. O caminho de acordar por `TERMINATED` roda a mesma máscara de `Waiters`, e não foi exercido.

## Por que foi aceito

Sem `process_create` nenhum processo espera outro: o teste exigiria um segundo caminho só para isto.

## Gatilho

O `process_create` entrar: o primeiro pai que espera o filho exercita o caminho, e o teste dele tem de esperar `TERMINATED` bloqueado.

## Como pagar

No programa de teste da parte 3, o pai cria o filho, espera `TERMINATED` com prazo longo e confere que acorda pela morte e não pelo prazo, como o `handles` faz com o canal.
