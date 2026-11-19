---
id: DEBT-0121
title: Reserva estrita só das páginas preguiçosas que o carregador cria
status: Open
date: 2026-11-18
trigger: O PR de VMO e VMAR como objetos (`vmo_create`, `vmar_map`) da fatia 4c
paid_date:
---

# DEBT 0121 — Reserva estrita só das páginas preguiçosas que o carregador cria

## O que falta

`commit::reserve` confere as páginas preguiçosas da pilha e do `.bss`, mais as tabelas que elas pedem (uma por 512 páginas e mais uma), contra os frames livres na criação do processo, e a falta de memória vira erro de `spawn`. Falta o resto da decisão 9 do SDD 17: `NoMemory` em `vmo_create` e `vmar_map`, o campo de política por processo (o modo heurístico do adaptador Linux). Ficam de fora da promessa: o que o kernel aloca depois (heap, pilhas), as tabelas das páginas que nascem mapeadas (imagem e argumentos), dois processos que criam ao mesmo tempo (cada um vê os frames livres antes do outro usar) e a página zero. Um processo pode morrer na falta de página por falta de frame (a linha da morte diz `out of memory`).

## Por que foi aceito

As chamadas que dão `NoMemory` não existem ainda. O contador já tem a forma final (promete, devolve ao ganhar frame ou ao morrer) e é global: um processo reserva a RAM inteira (o limite que a decisão 9 já descreve, que só os grupos de recursos da fase 9 fecham).

## Gatilho

O PR de VMO e VMAR como objetos (`vmo_create`, `vmar_map`) da fatia 4c.

## Como pagar

Chamar `commit::reserve` em `vmo_create` e `vmar_map` com o tamanho pedido, guardar o modo no `Process` e somar tabelas de página e estruturas do kernel por processo à promessa.
