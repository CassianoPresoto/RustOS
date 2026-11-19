---
id: DEBT-0121
title: Reserva estrita sem o modo por processo e sem o que o kernel aloca
status: Open
date: 2026-11-18
trigger: O adaptador Linux da fase 5 pedir o modo heurístico, ou o primeiro processo que morra de falta de frame por alocação do kernel que a promessa não cobre
paid_date:
---

# DEBT 0121 — Reserva estrita só das páginas preguiçosas que o carregador cria

## O que falta

`vmo_create` promete as páginas do VMO e `vmar_map` as tabelas do mapeamento (`NoMemory` na chamada), o carregador do kernel promete a pilha e o `.bss`. Falta o campo de política por processo (o modo heurístico do adaptador Linux). Ficam de fora da promessa: o que o kernel aloca depois (heap, pilhas de kernel), as tabelas das páginas que nascem mapeadas (imagem e argumentos do carregador do kernel), dois processos que prometem ao mesmo tempo (cada um vê os frames livres antes do outro usar) e a página zero. Um processo pode morrer na falta de página por falta de frame (a linha da morte diz `out of memory`).

## Por que foi aceito

O contador tem a forma final (promete, devolve ao ganhar frame, ao desmapear ou ao morrer) e é global: um processo reserva a RAM inteira (o limite que a decisão 9 já descreve, que só os grupos de recursos da fase 9 fecham).

## Gatilho

O adaptador Linux da fase 5 pedir o modo heurístico, ou o primeiro processo morto por falta de frame que a promessa não via.

## Como pagar

Guardar o modo no `Process`, e somar as pilhas de kernel e as tabelas das páginas que nascem mapeadas à promessa.
