---
id: DEBT-0156
title: Metade alta do kernel sem remoção depois do boot, premissa das etiquetas
status: Open
date: 2026-11-23
trigger: O primeiro `unmap` ou troca de permissão de endereço da metade alta (kernel) depois do boot
paid_date:
---

# DEBT 0156 — Metade alta do kernel sem remoção depois do boot

## O que falta

Com etiqueta de TLB, a metade alta (kernel) das raízes de processo é compartilhada por ponteiro, e a entrada
do kernel fica em cada etiqueta. Um `invlpg` (ou o *shootdown*) só invalida a etiqueta carregada na CPU; uma
entrada da metade alta removida depois do boot deixaria tradução velha nas outras etiquetas, sem invalidação
que a cubra. Hoje o kernel só **acrescenta** na metade alta depois do boot (pilhas novas); nenhum caminho remove.

## Por que foi aceito

A premissa vale no código atual e a fatia não muda isso. Pagar agora exigiria um `INVPCID` tipo 0 em todas as
etiquetas de cada CPU, ou o `PGE` global nas páginas do kernel, sem caso que o peça.

## Gatilho

O primeiro caminho do kernel que remove ou muda permissão de uma entrada da metade alta depois do boot.

## Como pagar

Ou tornar as entradas da metade alta globais (`G`, com `CR4.PGE`, que hoje o `mmu.rs` recusa), ou invalidar
em todas as etiquetas (`INVPCID` tipo 0 por etiqueta, ou tipo 2) no ponto da remoção, com o *shootdown* a todas as CPUs.
