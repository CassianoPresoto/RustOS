---
id: DEBT-0140
title: A ABI não diz o tamanho de um VMO, e o `init` mede o initramfs por tentativa de mapeamento
status: Open
date: 2026-11-19
trigger: O `object_get_info` entrar na ABI (um programa precisar do tamanho de um VMO ou de outra propriedade), ou o initramfs passar de 100 MiB
paid_date:
---

# DEBT 0140 — A ABI não diz o tamanho de um VMO, e o `init` mede o initramfs por tentativa de mapeamento

## O que falta

`rt::vm::Archive::open` acha o tamanho do VMO físico do initramfs mapeando 1, 2, 4... páginas até a `vmar_map` dar `OutOfRange` e depois por busca binária: cerca de 2 log2(N) chamadas (e desmapeamentos) uma vez por `init`. A mensagem de boot só leva o `kind` dos handles e o contrato não tem `vmo_get_size`.

## Por que foi aceito

Acrescentar chamada ou campo é caso 6 do `AGENTS.md` (contrato que o mundo de fora enxerga), e a medida custa microssegundos.

## Gatilho

O `object_get_info` entrar na ABI, ou o initramfs passar de 100 MiB (a busca percorre os tamanhos).

## Como pagar

Por uma decisão do dono: `object_get_info` (como o Zircon) ou o tamanho no corpo da mensagem de boot. O `Archive::open` passa a ler o tamanho.
