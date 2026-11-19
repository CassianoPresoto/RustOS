---
id: DEBT-0133
title: O fim de processo que o `run` espera vive numa tabela de 16 posições
status: Open
date: 2026-11-19
trigger: Mais de 16 processos poderem terminar entre o `spawn` e a espera de um `run`
paid_date:
---

# DEBT 0133 — O fim de processo que o `run` espera vive numa tabela de 16 posições

## O que falta

`programs::finished` guarda como cada processo terminou num anel de 16 entradas, e o `run` em primeiro plano procura o pid dele ali. Se mais de 16 processos terminassem depois do `spawn` e antes de a tarefa do shell acordar, a entrada seria sobrescrita e a espera não voltaria. Não acontece com um shell serial e processos em segundo plano que o usuário lança na mão.

## Por que foi aceito

A alternativa certa é esperar pelo `TERMINATED` do `ProcessObj`, com o evento guardado no objeto, e o objeto `Process` da parte 3 da 4c já levanta `TERMINATED` e guarda o evento (`LAST_EVENT`). O anel custa 512 bytes de estática.

## Gatilho

O primeiro programa que lance mais de 16 processos curtos.

## Como pagar

`Spawned` leva o `Arc<Process>`, o `Process` guarda o evento de saída, e o `run` espera `TERMINATED` nele (o mesmo `Waiters`); o anel e `programs::finished` saem.
