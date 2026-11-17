---
id: DEBT-0023
title: aarch64 sem SCTLR_EL1.WXN
status: Open
date: 2026-11-17
trigger: O primeiro código de usuário (fase 4), ou o primeiro mapeamento criado depois do boot que possa ser gravável e executável
paid_date:
---

# DEBT 0023 — aarch64 sem WXN

## O que falta

`SCTLR_EL1.WXN` faz a CPU tratar toda página gravável como não executável, mesmo que a
tabela diga o contrário. O kernel não o liga. O W^X de hoje vale pela conferência
das tabelas e pelo `fault text`, não por uma segunda trava da CPU.

## Por que foi aceito

Ligá-lo antes de a troca de tabelas depende de o Limine mapear a imagem sem W+X, e o
x86_64 não tem equivalente. As tabelas são conferidas no boot.

## Gatilho

O primeiro código de usuário (fase 4), ou qualquer mapeamento criado depois do boot
que o autoteste não veja.

## Como pagar

Ligar o bit em `platform::harden` do aarch64 depois de `Vm::activate`, e acrescentar
`wxn` à linha `protections:`.
