---
id: DEBT-0073
title: Limpeza da pilha do kernel na volta ao usuário para na primeira faixa limpa de 128 bytes
status: Open
date: 2026-11-18
trigger: O primeiro quadro de kernel no caminho de uma syscall ou falta de usuário com mais de 128 bytes de locais sem escrever, ou a medida da fase 4f em metal
paid_date:
---

# DEBT 0073 — Limpeza da pilha do kernel na volta ao usuário para na primeira faixa limpa de 128 bytes

## O que falta

A limpeza varre a pilha da tarefa para baixo, a partir de 256 bytes abaixo do quadro, e repinta o que
achou até encontrar 16 palavras (128 bytes) seguidas ainda pintadas. Um quadro de kernel com mais que
isso de locais que a chamada não escreveu esconde o que está abaixo dele: a limpeza para ali e o
resíduo fundo fica na pilha até o próximo uso. Hoje nenhum quadro do caminho de usuário é assim
(`debug_write` e `random_get` ficam fora de linha para o `nop` não carregar os buffers deles), e o
autoteste planta 2 KiB e confere que somem.

## Por que foi aceito

A limpeza exata (varrer do fundo até a primeira palavra suja, a marca d'água de `first_unpainted`)
custa a pilha inteira a cada volta; medida no aarch64 sob HVF, a heurística custa de 10 a 17 ns sobre
uma syscall nula de 37 a 50 ns ([SDD 07](../sdd/07_desempenho_e_recursos.md)). O Linux rastreia o
ponto mais fundo com instrumentação do compilador, que o RustOS não tem.

## Gatilho

O primeiro quadro no caminho de usuário com mais de 128 bytes de locais sem escrever, ou a medida
da 4f em metal, o que vier antes.

## Como pagar

Rastrear o `sp` mais fundo por tarefa (a entrada de syscall e a de exceção o guardam, ou o `arch/`
compara o `sp` de cada IRQ) e limpar só até ele, sem varrer; ou subir a faixa limpa com o número.
