---
id: DEBT-0006
title: Memória do bootloader não volta para o alocador
status: Open
date: 2026-11-17
trigger: O primeiro benchmark de memória ociosa contra o Linux
paid_date:
---

# DEBT 0006 — Memória do bootloader não volta para o alocador

## O que falta

As regiões `BootloaderReclaimable` (respostas do Limine, tabelas de página dele,
a pilha de boot) ficam fora do alocador de frames para sempre. Na QEMU com 256 MiB
elas são parte do `other` do relatório de boot.

## Por que foi aceito

O kernel ainda lê as respostas do Limine (`LimineBoot`) e roda na pilha e nas
tabelas dele. Liberar exige copiar o que se usa e ter tabelas e pilha próprias,
que só existem na fase 2.

## Gatilho

O primeiro benchmark de memória ociosa contra o Linux: sem isso, o RustOS aparece
com menos memória livre do que tem.

## Como pagar

Na fase 2: copiar o mapa de memória para estrutura do kernel, trocar para tabelas e
pilha próprias e então liberar as regiões `BootloaderReclaimable` no alocador.
