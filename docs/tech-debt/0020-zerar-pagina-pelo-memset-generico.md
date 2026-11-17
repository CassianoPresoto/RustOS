---
id: DEBT-0020
title: Zerar memória ao liberar usa o memset genérico, 5x mais lento que o do host
status: Open
date: 2026-11-17
trigger: O primeiro caminho quente que zere página inteira (page fault de usuário, na fase 4), ou o primeiro benchmark de fork, exec ou mmap contra o Linux
paid_date:
---

# DEBT 0020 — Zerar página pelo memset genérico

## O que falta

`DirectMap::zero` usa `write_bytes`, que vira o `memset` do `compiler_builtins`. No
aarch64 sob HVF, zerar custa cerca de 39 ns por KiB (4 KiB: +160 ns por par
alocar+liberar), contra cerca de 7 ns por KiB no host ([SDD 07](../sdd/07_desempenho_e_recursos.md)).
A causa não foi investigada: é a hipótese de o `memset` do alvo, sem SIMD, gravar de
8 em 8 bytes.

## Por que foi aceito

Hoje só o heap zera, e o kernel quase não aloca depois do boot. O número é o do
mecanismo ligado, como o [guidelines 09](../guidelines/09_seguranca.md) pede.

## Gatilho

O primeiro caminho quente que zere página inteira (page fault de usuário, na fase 4),
ou o primeiro benchmark de fork, exec ou mmap contra o Linux.

## Como pagar

Conferir o código gerado do `memset` no alvo e, se for o caso, zerar em `arch/` com
`dc zva` (aarch64) ou `rep stosb` com ERMS (x86_64) atrás de uma porta do `hal`,
com o antes e o depois no SDD 07.
