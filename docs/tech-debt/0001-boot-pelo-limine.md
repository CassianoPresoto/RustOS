---
id: DEBT-0001
title: Boot depende do Limine, binário de terceiros
status: Open
date: 2026-11-17
trigger: O tempo do bootloader passar a ser a maior parcela do boot medido em metal, ou o primeiro alvo sem UEFI
paid_date:
---

# DEBT 0001 — Boot depende do Limine, binário de terceiros

## O que falta

O kernel não sobe sozinho: o `cargo xtask image` baixa o Limine 12.10.0 (pacote
`limine-binary.tar.gz` da release no GitHub, SHA-256 fixado em
`xtask/src/limine.rs`) e usa o `BOOTX64.EFI`/`BOOTAA64.EFI` dele. A assinatura GPG
do pacote não é conferida, só o hash.

## Por que foi aceito

O Limine entrega a mesma interface nas duas ISAs (e em riscv64 e loongarch64):
modo longo/EL1, paginação ligada, mapa direto, mapa de memória. Isso deixou a fase
0 sem nenhuma linha de assembly de entrada. O protocolo foi escrito aqui
(`boot/limine`), sem crate, então trocar de bootloader é escrever outro adaptador
que devolva `hal::BootInfo` — o núcleo não muda.

## Gatilho

Quando o boot for medido em metal e o Limine for a maior parcela do tempo até o
kernel (linha `boot timeline`), ou quando aparecer um alvo sem UEFI (placa que só
entra por U-Boot/DTB).

## Como pagar

Um `boot/uefi` próprio, compilado para `x86_64-unknown-uefi` e
`aarch64-unknown-uefi` (alvos Tier 2), que monte as tabelas, saia dos boot
services e entre no kernel com o mesmo `hal::BootInfo`.
