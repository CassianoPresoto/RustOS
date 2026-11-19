---
id: DEBT-0130
title: `run` não passa argumentos ao programa (`argc` é sempre 0)
status: Open
date: 2026-11-19
trigger: O primeiro programa de `user/` que precise de argumentos, ou o `process_create` da parte 3 da 4c (quem cria o filho monta a pilha dele)
paid_date:
---

# DEBT 0130 — `run` não passa argumentos ao programa (`argc` é sempre 0)

## O que falta

O shell aceita `run <programa> [&]` e recusa palavra a mais ("programs take no arguments yet"). O carregador monta a pilha inicial com `argc = 0`, `argv` e `envp` vazios (`InitialStack::build`), e `process::spawn` não recebe argumentos.

## Por que foi aceito

O M1 pede listar, rodar e ver o processo; os quatro programas de demonstração não leem argumento. Passar `argv` muda `core/hal` (`InitialStack`, com teto de bloco e teste de layout), o carregador, o `Source` do processo e o `user/rt` (`Env::args`), e o `process_create` da parte 3 da 4c mexe no mesmo caminho: fazer os dois de uma vez evita conflito.

## Gatilho

O primeiro programa que precise de argumento (um `count N`, um `cat arquivo`), ou o PR de `process_create`. O layout é o do psABI já descrito no SDD 17 ("Pilha inicial"): ponteiros de `argv` terminados em zero depois de `argc`, e as cadeias acima do auxv.

## Como pagar

`InitialStack::build` recebe as palavras e põe as cadeias no bloco (o teto `INITIAL_STACK_MAX` sobe); `loader::map_stack` e `process::populate` as repassam; `Host::run_program` ganha o vetor; o `Env` do `rt` expõe `args()`. Teste no host do layout com 0, 1 e muitos argumentos e com cadeia que não cabe.
