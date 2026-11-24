---
id: DEBT-0159
title: Framebuffer com outra ordem de canais fica sem o VMO do usuário
status: Open
date: 2026-11-24
trigger: O primeiro firmware ou máquina-alvo que entregue a tela com os canais fora de vermelho 16, verde 8, azul 0 (8 bits cada) quando um programa precisar da tela
paid_date:
---

# DEBT 0159 — Framebuffer com outra ordem de canais fica sem o VMO do usuário

## O que falta

O formato `XRGB8888` da mensagem de boot v2 ([SDD 18](../sdd/18_m2_doom_nativo.md#como-entrou-fatia-1a)) só descreve a ordem
vermelho 16, verde 8, azul 0. O bootloader aceita qualquer canal de 8 bits com deslocamento até 24. Numa tela com outra
ordem, o programa do shell não recebe o `kind` 5, e o console segue desenhando.

## Por que foi aceito

O QEMU (x86_64 pelo Limine, aarch64 pelo ramfb) entrega a ordem XRGB, e o M2 só precisa dela. Converter a ordem no
kernel ou descrever a ordem no formato é trabalho sem caso de uso hoje.

## Gatilho

O primeiro firmware ou hardware que entregue a tela com outra ordem de canais, com um programa de usuário precisando da tela.

## Como pagar

Acrescentar um código de `format` no bloco de 16 bytes (que descreva a ordem dos canais) e ajustar `Vmo`/`framebuffer.rs`
para aceitá-lo, ou converter no programa. Cada caso muda o contrato da [ABI](../sdd/17_processos_isolamento_e_syscalls.md): o
caso é do `AGENTS.md`, e entra como decisão.
