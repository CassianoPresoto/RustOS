---
id: DEBT-0070
title: A NVRAM do EDK2 do aarch64 é montada à mão no xtask, com o layout do EDK2 de 2024
status: Open
date: 2026-11-18
trigger: O `boot timeline:` do aarch64 mostrar `firmware` acima de 1 s, ou a troca do EDK2 por uma versão com outro layout de NVRAM
paid_date:
---

# DEBT 0070 — A NVRAM do EDK2 do aarch64 é montada à mão no xtask

## O que falta

O `xtask/src/nvram.rs` escreve o armazém de variáveis do EDK2 do aarch64 com `Timeout = 0`: três blocos de 256 KiB, cabeçalho de volume,
cabeçalho do armazém autenticado e uma variável. Foi conferido byte a byte contra o `edk2-arm-vars.fd` do EDK2 `stable202408` da QEMU 11.1.3
(Homebrew). Se o EDK2 de outro pacote (o do runner do CI, `qemu-efi-aarch64`) usar outro layout, ele refaz a NVRAM e volta a esperar o
`Timeout` padrão de 5 s antes do Limine, sem erro: só o `firmware` do `boot timeline:` denuncia.

## Por que foi aceito

Medido: com a NVRAM montada o `firmware` cai de 5,1 s para 0,10 s ([SDD 07](../sdd/07_desempenho_e_recursos.md#tempo-de-boot)). O CI roda
o aarch64 em TCG e não tem teto de tempo do firmware; o `make smoke` passa nos dois casos, só mais devagar.

## Gatilho

O `boot timeline:` do aarch64 mostrar `firmware` acima de 1 s (HVF) ou de 2 s (TCG), ou a troca do EDK2 por uma versão com outro layout.

## Como pagar

Ler o layout do `edk2-arm-vars.fd` do pacote achado em vez de fixá-lo (copiar o modelo e acrescentar a variável), ou pôr um teto no
`firmware` do `smoke-fast` do aarch64 para o desvio quebrar o CI em vez de passar calado.
