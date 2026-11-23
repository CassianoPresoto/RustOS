---
id: DEBT-0131
title: Sem Ctrl-C nem `kill`: o `run` em primeiro plano espera sem prazo
status: Open
date: 2026-11-19
trigger: O primeiro programa de `user/` que não termine sozinho, ou o `kill <pid>` do shell
paid_date:
---

# DEBT 0131 — Sem Ctrl-C nem `kill`: o `run` em primeiro plano espera sem prazo

## O que falta

Um programa que trava (laço sem fim, espera que nunca chega) não morre sozinho: a CPU o preempta e o sistema segue, mas `run <programa>` em primeiro plano só volta quando o processo termina, e o shell fica sem prompt. Ctrl-C (0x03) chega ao shell como byte de entrada, mas nada o lê durante a espera, e o `run` não guarda handle do processo que lança: o `process_kill` existe (parte 3 da 4c), mas o shell não tem por onde chamá-lo. Os programas de demonstração terminam sozinhos, e o `crash` morre pela falta de página.

## Por que foi aceito

Matar de fora exigia parar threads em outras CPUs, que a parte 3 da 4c entregou; ligar o shell a isso é trabalho dele.

## Gatilho

Um programa de `user/` que possa não terminar, ou o `kill <pid>` do shell.

## Como pagar

A espera do `run` passa a acordar também com a entrada: Ctrl-C solta o shell (o processo vira de segundo plano) e `kill <pid>` chama `Process::kill` (o `spawn` devolve o `Arc<Process>`). O `ps` já mostra o pid. Teste no smoke: um programa que gira, `run` e Ctrl-C.
