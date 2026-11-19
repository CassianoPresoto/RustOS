---
id: DEBT-0133
title: O fim de processo que o `run` espera vive numa tabela de 16 posições
status: Open
date: 2026-11-19
trigger: Mais de 16 processos poderem terminar entre o `spawn` e a espera de um `run`, ou o `process_create` da parte 3 da 4c (a espera passa a ser por handle)
paid_date:
---

# DEBT 0133 — O fim de processo que o `run` espera vive numa tabela de 16 posições

## O que falta

`programs::finished` guarda como cada processo terminou num anel de 16 entradas, e o `run` em primeiro plano procura o pid dele ali. Se mais de 16 processos terminassem depois do `spawn` e antes de a tarefa do shell acordar, a entrada seria sobrescrita e a espera não voltaria. Não acontece com um shell serial e processos em segundo plano que o usuário lança na mão.

## Por que foi aceito

A alternativa certa é esperar pelo `TERMINATED` do `ProcessObj`, com o evento guardado no objeto, e isso depende da parte 3 da 4c (que mexe em `process.rs` e em `object.rs`). O anel custa 512 bytes de estática.

## Gatilho

O `process_create` da parte 3 entrar, ou o primeiro programa que lance mais de 16 processos curtos.

## Como pagar

`Process` entrega o `Arc<ProcessObj>` ao `Spawned`, o `ProcessObj` guarda o evento de saída, e o `run` espera `TERMINATED` nele (o mesmo `Waiters`); o anel e `programs::finished` saem.
