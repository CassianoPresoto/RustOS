---
id: DEBT-0079
title: IRQ no meio da cópia de usuário roda com `EFLAGS.AC` ligado no x86_64
status: Open
date: 2026-11-18
trigger: O primeiro boot do x86_64 em metal ou com KVM, ou a leitura da seção do SDM sobre a entrega de interrupção e o `AC`
paid_date:
---

# DEBT 0079 — IRQ no meio da cópia de usuário roda com `EFLAGS.AC` ligado no x86_64

## O que falta

O `usercopy` abre a janela do SMAP com `stac` e a fecha com `clac`. Uma IRQ que chega entre os
dois entra no kernel com `AC = 1`: o tratador e a troca de tarefa que ele faça rodam com o SMAP
desarmado até o retorno. Que a CPU não limpe o `AC` ao entregar a interrupção está **NÃO VERIFICADO**
no SDM; o Linux faz `clac` nos stubs, o RustOS não.

## Por que foi aceito

A janela é de poucas instruções, a cópia é de no máximo 512 bytes, e o código do kernel que roda
nela é o tratador de IRQ, não código de usuário. A perda é a proteção do SMAP naquele intervalo.

## Gatilho

O primeiro boot em metal ou com KVM (onde o SMAP é de verdade), ou a leitura do SDM sobre o `AC`.

## Como pagar

`clac` na entrada comum dos stubs que vêm de anel 0, só com SMAP (uma chave estática, porque
`clac` sem SMAP é `#UD`), ou conferir no SDM que a CPU o limpa.
