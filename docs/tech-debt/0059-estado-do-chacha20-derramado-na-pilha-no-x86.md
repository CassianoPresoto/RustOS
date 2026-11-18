---
id: DEBT-0059
title: Palavras do estado do ChaCha20 derramadas na pilha pelo `permute` no x86_64
status: Open
date: 2026-11-17
trigger: A primeira volta do kernel para código de usuário (a syscall da fase 4), ou o primeiro `copy_to_user` que leia a pilha do kernel
paid_date:
---

# DEBT 0059 — Estado do ChaCha20 derramado na pilha no x86_64

## O que falta

O estado do ChaCha20 e o gerador não deixam mais cópia da chave na pilha (o autoteste
`key residue:` do boot acha zero, [SDD 09](../sdd/09_seguranca.md)). Sobra o que o
autoteste não enxerga: no x86_64 `release`, o `permute` não cabe nos 16 registradores e
derrama palavras do estado de trabalho, no meio das rodadas, num quadro de 40 bytes (o
desmontado mostra ao menos um `movl` de palavra do estado para `(%rsp)`). As rodadas são invertíveis, então uma palavra intermediária ainda diz algo da
chave. Essas palavras não têm padrão reconhecível e `Drop` nenhum as alcança.

## Por que foi aceito

Zerar a pilha abaixo de cada chamada criptográfica dentro de `core/crypto` custa alguns
nanossegundos por pedido e só cobre a profundidade que a chamada alcançou. O lugar certo
é a saída do kernel para o usuário, que limpa tudo de uma vez. Hoje só roda código do
kernel.

## Gatilho

A primeira volta do kernel para código de usuário (retorno de syscall na fase 4), ou o
primeiro `copy_to_user` que leia a pilha do kernel.

## Como pagar

Limpar a pilha do kernel na saída para o usuário (o que o Linux faz com
`CONFIG_GCC_PLUGIN_STACKLEAK` e `init_stack`), medindo o custo no retorno de syscall.
Antes, conferir se o aarch64 também derrama em `release-small`.
