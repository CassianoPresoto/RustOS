---
id: DEBT-0026
title: Troca de tabela do aarch64 sem a leitura do Arm ARM nem a conferência do MAIR
status: Open
date: 2026-11-17
trigger: O primeiro boot em metal aarch64, ou a leitura do Arm ARM nas seções de TLB e de MAIR
paid_date:
---

# DEBT 0026 — Troca de tabela do aarch64 sem conferência

## O que falta

`Aarch64Mmu::activate` (`arch/aarch64/src/mmu.rs`) foi escrito sem o Arm ARM aberto, e
três pontos dependem dele:

1. **`tlbi` antes do `msr ttbr1_el1`.** O primeiro `tlbi vmalle1` roda antes de a raiz
   trocar, e a CPU pode recarregar tradução da tabela antiga na janela até o `msr`; o
   `tlbi` depois do `msr` cobre isso. Falta decidir se o primeiro é necessário ou se
   a ordem certa é só `msr`, `isb`, `tlbi`, `dsb`, `isb`.
2. **Conflito de tamanhos de TLB.** A troca assume que bloco de 2 MiB e página de 4 KiB
   para o mesmo endereço em tabelas diferentes não vivem juntos, e por isso há dois
   flushes. O que o Arm ARM exige (*TLB conflict abort*, `BBM`) não foi lido.
3. **`MAIR_EL1` índice 1 não é conferido em execução.** O framebuffer usa o atributo 1
   "do Limine" (PROTOCOL.md, base revisão 4); `prepare_space` só escreve o índice 7 e
   não lê o 1 para ver se é o tipo esperado.

## Por que foi aceito

A QEMU (TCG e HVF) não modela TLB nem tipo de memória de modo que um erro aqui apareça;
sem metal, não há o que medir.

## Gatilho

O primeiro boot em metal aarch64, ou a leitura do Arm ARM (*Break-before-make*, *TLB
conflict abort*, `MAIR_EL1`), o que vier antes.

## Como pagar

Ler as três seções, ajustar a sequência de `activate` (um flush ou dois, antes ou
depois do `msr`) e fazer `prepare_space` recusar o boot se o `MAIR_EL1.Attr1` não for o
tipo de framebuffer que o código supõe. O `x86_64` não entra: `require_features` já lê
`CR4.PGE` e recusa o espaço novo com ele ligado.
