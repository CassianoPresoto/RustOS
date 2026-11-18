---
id: DEBT-0100
title: KPTI não implementado, e `kpti=on|off|auto` só entra no relatório
status: Open
date: 2026-11-18
trigger: O PR do KPTI (fatia 4e), que cria a raiz de usuário com o trampolim e o byte de teste na entrada e na saída
paid_date:
---

# DEBT 0100 — KPTI não implementado, e `kpti=on|off|auto` só entra no relatório

## O que falta

A linha `mitigations:` diz `kpti off (not implemented, would be on)` na CPU que a detecção marca
com Meltdown (x86_64: sem `RDCL_NO` e fora das listas da AMD e da Hygon; aarch64: sem `CSV3` e fora
da lista de CPUs imunes do Linux). O kernel mapeia a metade do kernel em toda raiz de usuário.
A chave `kpti=on|off|auto` da linha de comando já é lida e só muda o texto da linha.

## Por que foi aceito

Esta fatia entrega a detecção, o relatório e o clamp do Spectre v1. O KPTI mexe na entrada, na
saída e nas tabelas, e entra com o custo medido, num PR próprio
([SDD 17](../sdd/17_processos_isolamento_e_syscalls.md#o-trampolim-e-o-caminho-pronto-para-kpti)).

## Gatilho

O PR do KPTI.

## Como pagar

O trampolim e as duas raízes por processo do SDD 17; trocar `Enabled::kpti` por `Switch::On` (ou
`Untested`, se o metal for o único que prova) em `kernel/src/mitigation.rs`; medir a syscall nula com
`kpti=on` e `kpti=off`.
