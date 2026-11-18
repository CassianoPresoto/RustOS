---
id: DEBT-0078
title: Exceção síncrona de EL1h do aarch64 segue na pilha de falhas, sem o teste de estouro da pilha da tarefa
status: Open
date: 2026-11-18
trigger: A primeira falta de página de usuário resolvida dentro do kernel (a 4f: página zero e COW), ou o primeiro `brk` ou *fixup* que precise reescalonar
paid_date:
---

# DEBT 0078 — Exceção síncrona de EL1h do aarch64 segue na pilha de falhas, sem o teste de estouro da pilha da tarefa

## O que falta

O [SDD 17](../sdd/17_processos_isolamento_e_syscalls.md#aarch64-svc) manda toda exceção síncrona
de EL1h entrar na pilha da tarefa (`SP_EL1`), com o teste de estouro do `kernel_ventry` do Linux, e
só o estouro ir para a pilha de falhas. A 4a mudou o que o desenho exigia e nada além: o
`vector_fault` lê a pilha de falhas do bloco por CPU (`TPIDR_EL1`) em vez do `SP_EL0`, que agora é
a pilha do usuário. Falha de EL1h continua na pilha de falhas, que reinicia do topo a cada uma.

## Por que foi aceito

As exceções de EL1h que voltam hoje (`brk` e o *fixup* do `usercopy`) não reagendam nem dormem, e a
pilha de falhas que reinicia do topo serve a quem volta direto. Mudar a ordem pede o teste de
estouro sem registrador livre e a pilha alinhada ao dobro do tamanho.

## Gatilho

A falta de página de usuário resolvida no kernel (a 4f), ou a primeira exceção de EL1h que precise
trocar de tarefa antes de voltar.

## Como pagar

Entrar na pilha da tarefa com o teste de estouro, e só no estouro ir à pilha de falhas, como o
`__bad_stack` do Linux; o SDD 17 descreve o desenho.
