---
id: DEBT-0131
title: Sem Ctrl-C nem `kill`: o `run` em primeiro plano espera sem prazo
status: Open
date: 2026-11-19
trigger: O `process_kill` (parte 3 da 4c) ou o primeiro programa de `user/` que não termine sozinho
paid_date:
---

# DEBT 0131 — Sem Ctrl-C nem `kill`: o `run` em primeiro plano espera sem prazo

## O que falta

Um programa que trava (laço sem fim, espera que nunca chega) não morre sozinho: a CPU o preempta e o sistema segue, mas `run <programa>` em primeiro plano só volta quando o processo termina, e o shell fica sem prompt. Ctrl-C (0x03) chega ao shell como byte de entrada, mas nada o lê durante a espera, e o kernel não tem como matar um processo de fora da tarefa dele (`ProcessKill` devolve `NotSupported`). Os programas de demonstração terminam sozinhos, e o `crash` morre pela falta de página.

## Por que foi aceito

Matar de fora exige o encerramento de uma thread que roda em outra CPU ou está bloqueada (DEBT-0111), que é a parte 3 da 4c. Antes disso qualquer atalho seria um segundo caminho de morte do processo.

## Gatilho

A `process_kill` da parte 3 da 4c entrar, ou um programa de `user/` que possa não terminar.

## Como pagar

A espera do `run` passa a acordar também com a entrada: Ctrl-C solta o shell (o processo vira de segundo plano) e `kill <pid>` chama o `process_kill`. O `ps` já mostra o pid. Teste no smoke: um programa que gira, `run` e Ctrl-C.
