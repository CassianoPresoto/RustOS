---
id: DEBT-0063
title: Núcleo secundário sem vetores nem serial precoce até o init próprio
status: Open
date: 2026-11-17
trigger: O primeiro boot que trave com `cpus: N online` abaixo do pedido e nenhuma linha de falha do núcleo que não subiu, ou o primeiro boot em metal com mais de um núcleo
paid_date:
---

# DEBT 0063 — Núcleo secundário sem vetores nem serial precoce até o init próprio

## O que falta

`early::init` instala vetores e serial só na CPU de boot. Um núcleo secundário entra
(`smp::ap_entry`) com a IDT do Limine (limite 0, no x86_64: a exceção vira triple fault) ou o
`VBAR_EL1` indefinido (aarch64), e, no aarch64, sem o `TTBR0_EL1` da serial precoce nem a
janela de dispositivos, que só existe nas tabelas do kernel que ele troca em `secondary_early`.
Pânico ou exceção nesse trecho não escreve nada; o BSP só vê o núcleo `silent` no `cpus:`.

## Por que foi aceito

O trecho é curto (instala o `PerCpu`, liga as proteções, troca as tabelas) e o BSP já relata o
núcleo que não respondeu. A saída precoce dele exigiria vetores e serial por CPU antes de
qualquer pilha própria.

## Gatilho

O primeiro boot que trave com `cpus: N online` abaixo do pedido e nenhuma linha de falha do
núcleo que não subiu, ou o primeiro boot em metal com mais de um núcleo.

## Como pagar

Chamar `platform::early_init` também na entrada de cada núcleo (a IDT e o `VBAR_EL1` são
carregados por CPU; o x86_64 só relê a IDT compartilhada) e, no aarch64, repetir o mapa do
`TTBR0_EL1` de `arch/aarch64/src/early.rs`, que hoje usa duas páginas da pilha da CPU de boot, num `static` por CPU (8 KiB) ou na pilha do Limine de cada uma.
