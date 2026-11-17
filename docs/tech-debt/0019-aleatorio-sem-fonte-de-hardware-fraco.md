---
id: DEBT-0019
title: Aleatório só com jitter do contador em CPU sem gerador de hardware
status: Open
date: 2026-11-17
trigger: O primeiro consumidor de segurança do aleatório (KASLR, canário, ASLR de usuário) rodando em CPU sem RDSEED, RDRAND nem RNDR
paid_date:
---

# DEBT 0019 — Aleatório só com jitter em CPU sem gerador de hardware

## O que falta

Sem `RDSEED`, `RDRAND` ou `RNDR`, a semente é o jitter do contador mais os marcos do
boot, e o boot diz `[rustos] random: jitter (weak)`. É o caso do aarch64 sob HVF no
Apple Silicon, onde `ID_AA64ISAR0_EL1.RNDR` vem zerado. O contador do aarch64 anda a
24 MHz (uma leitura a cada 41 ns), então cada amostra traz pouco ruído. Nenhuma
contabilidade de entropia diz quantos bits a semente vale.

## Por que foi aceito

Hoje só o comando `random` usa o gerador: nada de segurança depende dele ainda. O
jitter e os marcos nunca pioram o que o hardware deu ([SDD 09](../sdd/09_seguranca.md)).

## Gatilho

O primeiro consumidor de segurança do aleatório (KASLR, canário de pilha, ASLR de
usuário na fase 4) rodando em CPU sem gerador de hardware.

## Como pagar

Uma fonte que não seja o relógio da própria CPU: `EFI_RNG_PROTOCOL` do firmware antes
de sair dos serviços de boot (pede ao bootloader), um driver de `virtio-rng`, ou a
chamada SMCCC TRNG do Arm. Junto, reposição por tempo além da por volume, e a
contabilidade de entropia que o `rustos-crypto` não faz.
