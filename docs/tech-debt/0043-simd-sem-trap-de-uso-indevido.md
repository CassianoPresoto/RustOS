---
id: DEBT-0043
title: Tarefa sem SIMD que usa SIMD não é pega
status: Open
date: 2026-11-17
trigger: O primeiro código de usuário (fase 4), quando todo processo usa SSE
paid_date:
---

# DEBT 0043 — Tarefa sem SIMD que usa SIMD não é pega

## O que falta

Só a tarefa criada com `simd: true` tem área de FPU/SIMD e paga o `save` e o `restore`
da troca. Uma tarefa com `simd: false` que execute uma instrução SIMD não trapeia: no
x86_64 não há `CR0.TS` nem #NM, e no aarch64 o `CPACR_EL1.FPEN` deixa o acesso livre. Ela
corrompe o estado da tarefa com SIMD que rodou antes e o vaza para a que roda depois.

## Por que foi aceito

O kernel é soft-float e as tarefas dele declaram `simd` com acerto. Nenhuma tarefa de
fora do kernel existe, então o caso que a trap cobriria ainda não ocorre.

## Gatilho

O primeiro código de usuário (fase 4): todo processo usa SSE (o ABI do x86_64 o exige),
e um erro de declaração deixa de ser do kernel.

## Como pagar

`simd` vale `true` para toda tarefa de usuário, o que paga o custo da troca por tarefa; ou
trap no primeiro uso (`CR0.TS` com #NM, `CPACR_EL1.FPEN` com trap) que aloca a área e liga
o `simd`, como o Linux faz com o SVE. Medir o #NM contra o `XSAVEOPT` eager (SDD 07).
