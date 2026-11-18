---
id: DEBT-0070
title: Pilha de 16 KiB medida só com tarefas do kernel, sem syscall nem falta de usuário
status: Open
date: 2026-11-18
trigger: A primeira syscall ou falta de página de usuário rodando na pilha do kernel de uma thread (fase 4a)
paid_date:
---

# DEBT 0070 — Pilha de 16 KiB medida só com tarefas do kernel, sem syscall nem falta de usuário

## O que falta

A pilha de toda thread do kernel tem 4 páginas (16 KiB) porque o pico medido é de 2,2 a 2,5 KiB com tarefas do kernel, IRQ de timer, IPI e pânico ([SDD 15](../sdd/15_tarefas_e_preempcao.md#pico-de-pilha)). A fase 4 vai rodar syscalls, cópia usuário↔kernel e a falta de página de usuário nessa mesma pilha, e nada disso está no número.

## Por que foi aceito

A margem é de 6,4x o pico de tarefa e a guarda sem mapeamento transforma estouro em relatório: baixar de 8 para 4 páginas já economiza 16 KiB por tarefa, e ficar em 8 sem medida seria pagar por um caso que não existe.

## Gatilho

A primeira syscall ou a primeira falta de página de usuário que rode na pilha do kernel da thread, na 4a. Antes de ela entrar no PR, o pico se remede.

## Como pagar

Rodar `ps`, `bench` e o boot (linha `stacks:`) com a 4a, nas duas ISAs e em `dev` e `release`, com um programa de usuário que faça as syscalls e uma falta fatal. Se o pico de tarefa passar de 8 KiB (metade da pilha), subir para 8 páginas; se ficar abaixo de 4 KiB com a 4a completa, pode-se medir 2 páginas.
