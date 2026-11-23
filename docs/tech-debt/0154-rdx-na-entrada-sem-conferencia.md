---
id: DEBT-0154
title: O `rdx` zerado na entrada do x86_64 deixou de ser conferido pelo autoteste
status: Open
date: 2026-11-23
trigger: A fase 5 trazer o primeiro programa linkado com libc (glibc ou musl) no initramfs, ou qualquer mudança no `enter_user` do x86_64
paid_date:
---

# DEBT 0154 — O `rdx` zerado na entrada do x86_64 deixou de ser conferido pelo autoteste

## O que falta

O caso `hello` do blob conferia `rdx == 0` na primeira instrução, o contrato de registrador de entrada (SDD 17,
"Divergências"). Ao sair do blob, o `hello` virou função de `arch/x86_64-user` chamada depois do `_start` e do `main`
do `user/tests/cases`, e o compilador já pode ter usado `rdx` antes dela. Nenhum caso confere o registrador de entrada
hoje. O `enter_user` zera `rdx`, e uma regressão ali não é detectada por nenhum teste.

## Por que foi aceito

Conferir `rdx` na primeira instrução pediria o `_start` do `arch/x86_64-user` a fazer a conferência (ou a um caso em
`asm` puro antes do `main`), e o único consumidor desse contrato é a glibc, que ainda não existe no RustOS.

## Gatilho

A fase 5 trazer o primeiro programa linkado com libc (glibc ou musl) no initramfs, que lê `rdx` como `atexit`, ou
qualquer mudança no `enter_user` do x86_64. Ali a conferência volta, no `_start` do `arch/x86_64-user`, com um caso
novo do `user/tests/cases`.
