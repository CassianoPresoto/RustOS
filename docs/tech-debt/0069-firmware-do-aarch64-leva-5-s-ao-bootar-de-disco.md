---
id: DEBT-0069
title: O EDK2 do aarch64 leva 5,1 s antes do Limine ao bootar do disco
status: Open
date: 2026-11-18
trigger: O `make smoke` do aarch64 estourar o tempo no CI, ou o primeiro boot do aarch64 fora da QEMU
paid_date:
---

# DEBT 0069 — O EDK2 do aarch64 leva 5,1 s antes do Limine ao bootar do disco

## O que falta

Na QEMU `virt` com HVF, o `boot timeline:` do RustOS diz `firmware 5,1 s`: o tempo do reset até o Limine. O mesmo EDK2 leva
0,3 s a carregar o Linux por `-kernel` (fw_cfg), com ou sem o disco anexado. Não depende de `ramfb`, de `bootindex`, de `-boot menu=off`, do
tipo FAT nem de o disco ser só leitura (todas testadas). O tempo de parede do `make smoke` do aarch64 é 98% firmware (5,2 s contra 2 ms de kernel).

## Por que foi aceito

Não é do kernel: não há o que o RustOS faça, só a configuração da QEMU. A comparação com o Linux subtrai o firmware de cada caminho
([SDD 07](../sdd/07_desempenho_e_recursos.md#tempo-de-boot)).

## Gatilho

O `make smoke` do aarch64 estourar o tempo no CI, ou o primeiro boot do aarch64 fora da QEMU.

## Como pagar

Achar onde o EDK2 gasta: um EDK2 compilado com log de depuração na serial, uma imagem FAT em arquivo em vez do `vvfat`, e um `Timeout` do gerenciador de boot zerado numa NVRAM persistente.
