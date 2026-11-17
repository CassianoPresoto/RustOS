---
id: DEBT-0025
title: Cópias do estado do ChaCha20 e do gerador antigo ficam na pilha
status: Open
date: 2026-11-17
trigger: O primeiro código de usuário (fase 4)
paid_date:
---

# DEBT 0025 — Resíduo de chave na pilha

## O que falta

O gerador troca a chave a cada pedido (`Rng::fill_bytes`), mas o compilador deixa
cópias de material de chave em pilha que ninguém zera: o estado de 16 palavras que
`initial_state` monta e `permute` consome (`core/crypto/src/chacha20.rs`), o `Rng`
antigo que `reseed` substitui (`core/crypto/src/rng.rs`) e o `Rng` que `init` move para
a variável global (`kernel/src/random.rs`). `wipe` só alcança os buffers que o código
nomeia.

## Por que foi aceito

Hoje só roda código do kernel, que já lê qualquer memória. O resíduo só vira
vazamento quando outro domínio puder ler pilha que o kernel já largou, e isso ainda não
existe.

## Gatilho

O primeiro código de usuário (fase 4): a pilha do kernel reaproveitada passa a poder
vazar para ele (retorno de syscall, `copy_to_user` com padding, dump de pilha).

## Como pagar

Medir primeiro (olhar a pilha depois de `fill_bytes` num teste de kernel que varra a
região abaixo do `sp`). Depois, o estado do ChaCha20 e o `Rng` antigo em tipo com `Drop`
que chama `wipe`, e `permute` sobre o estado por referência, sem cópia. O `wipe` do
`random.rs` cobre a semente e o jitter, não o `Rng`.
