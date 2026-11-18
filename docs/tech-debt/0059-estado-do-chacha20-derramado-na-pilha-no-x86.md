---
id: DEBT-0059
title: Resíduo em `core/crypto` que o autoteste de pilha não enxerga
status: Open
date: 2026-11-17
trigger: A primeira volta do kernel para código de usuário (a syscall da fase 4), ou o primeiro `copy_to_user` que leia a pilha do kernel
paid_date:
---

# DEBT 0059 — Resíduo em `core/crypto` que o autoteste de pilha não enxerga

## O que falta

O estado do ChaCha20 e o gerador não deixam mais cópia da chave na pilha (o autoteste
`key residue:` do boot acha zero, [SDD 09](../sdd/09_seguranca.md)). Sobra o que o
autoteste não enxerga: no x86_64 `release`, o `permute` não cabe nos 16 registradores e
derrama palavras do estado de trabalho, no meio das rodadas, num quadro de 40 bytes (o
desmontado mostra ao menos um `movl` de palavra do estado para `(%rsp)`). As rodadas são invertíveis, então uma palavra intermediária ainda diz algo da
chave. Essas palavras não têm padrão reconhecível e `Drop` nenhum as alcança.

Também fora do alcance do autoteste: `gather_jitter` devolve o `[u8; 32]` por valor, e
`Rng::new` devolve o gerador por valor (cópia da chave no quadro de quem chama); o `kernel`
só usa `Rng::unseeded` no `init`, mas `benchmark` e os testes usam `new`. O gate cobre
`Words` e o `Rng` semeado com zero, não a chave real de `replenish`. O `v` do BLAKE2b já
é zerado como o `m`.

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
