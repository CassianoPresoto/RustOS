---
id: DEBT-0077
title: Falha do `iretq` por outra causa que `RIP` ou `RSP` não canônico não tem fixup
status: Open
date: 2026-11-18
trigger: A 4e ligar o KPTI (o `iretq` passa a rodar com o CR3 do usuário), ou a primeira falha de `iretq` que a conferência de `RIP` e `RSP` canônicos não pegue
paid_date:
---

# DEBT 0077 — Falha do `iretq` por outra causa que `RIP` ou `RSP` não canônico não tem fixup

## O que falta

O desenho do [SDD 17](../sdd/17_processos_isolamento_e_syscalls.md#exceções-vindas-do-anel-3)
trata o `#GP`, `#SS` e `#NP` do `iretq` no stub, comparando o `RIP` do quadro com o endereço da
instrução. A 4a implementou outra defesa: a saída para o usuário confere `RIP` e `RSP` canônicos
no quadro e, se algum não for, mata o processo antes do `iretq`. Cobre a única causa que o usuário
controla (`RSP` não canônico, por `mov rsp` e uma syscall ou IRQ). Se o `iretq` falhar por outra
causa, a CPU já trocou o `GS`, e o stub do `#GP` tomaria o `GS` do usuário.

## Por que foi aceito

Seletores e `RFLAGS` do quadro vêm do kernel ou da CPU, o `NT` é limpo na entrada (`FMASK` e a
porta de interrupção), e o `RIP` vem validado: nenhuma outra causa é alcançável hoje.

## Gatilho

A 4e (KPTI), ou a primeira causa de falha do `iretq` fora das duas conferidas.

## Como pagar

O *fixup* do SDD 17: nos stubs dos vetores 11, 12 e 13, comparar o `RIP` do quadro com o do
`iretq` antes de tocar no `GS`, e montar o `Fault` de usuário.
