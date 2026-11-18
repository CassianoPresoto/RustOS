---
id: DEBT-0083
title: Imagem de usuário sem ASLR
status: Open
date: 2026-11-18
trigger: O primeiro programa de usuário que não seja nosso, ou o início da fase 5
paid_date:
---

# DEBT 0083 — Imagem de usuário sem ASLR

## O que falta

O `init` é um `ET_EXEC` ligado em `0x40_0000` ([`user/link/<isa>.ld`](../../user/link/x86_64.ld)) e o
carregador o mapeia onde o linker o pôs. Só o topo da pilha é sorteado (28 bits de página); o código, os
dados e o `.bss` ficam sempre no mesmo endereço, então um programa com falha de memória o acha sem
adivinhar.

## Por que foi aceito

O único programa da 4b é o `init`, escrito pelo projeto, sem entrada hostil
([decisão 5](../sdd/17_processos_isolamento_e_syscalls.md#decisão-5--pie-de-usuário-agora-ou-depois)). O
`ET_DYN` pede relocação nos dois carregadores (kernel e `user/rt`) e o custo não paga um caso que ainda
não existe.

## Gatilho

O primeiro programa de usuário que não seja nosso, ou o início da fase 5 (a ABI Linux roda binário de
terceiros).

## Como pagar

Aceitar `ET_DYN` no `core/elf` (sem `PT_INTERP`), aplicar `R_X86_64_RELATIVE` (8) e `R_AARCH64_RELATIVE`
(1027) no carregador, ligar os programas como PIE (`-C relocation-model=pie`) e sortear a base com
`aslr_base`, como a pilha.
