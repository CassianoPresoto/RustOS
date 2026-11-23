---
id: DEBT-0141
title: O VMO nunca usa a página zero compartilhada: ler página nunca tocada gasta um frame
status: Open
date: 2026-11-19
trigger: A medida mostrar leitura esparsa de VMO grande entre os maiores consumidores de frame, ou o `vmo_create_child` entrar
paid_date:
---

# DEBT 0141 — O VMO nunca usa a página zero compartilhada: ler página nunca tocada gasta um frame

## O que falta

A falta numa região de VMO (`FaultPlan::MapObject`) cria o frame zerado **no VMO** em leitura e em escrita. A região anônima do carregador (pilha, `.bss`) segue com a página zero e a cópia dela. Um programa que lê um VMO de 64 MiB inteiro sem escrever gasta 16 384 frames; com a página zero gastaria um.

## Por que foi aceito

O VMO é compartilhado: com a página zero mapeada num espaço e o frame criado por outro, o primeiro ficaria vendo zero para sempre (o Zircon rastreia os mapeamentos para invalidar; o RustOS não os lista). A promessa da decisão 9 já cobre o gasto.

## Gatilho

A medida (SDD 07) mostrar leitura esparsa de VMO grande, ou o `vmo_create_child` (COW) entrar.

## Como pagar

Guardar no VMO a lista dos espaços que o mapeiam (ou um contador de mapeamentos) e, com um só mapeamento, usar a página zero na leitura e trocá-la na escrita como a região anônima; com mais de um, criar o frame.
