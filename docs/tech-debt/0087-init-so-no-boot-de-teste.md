---
id: DEBT-0087
title: O `init` só roda no boot de teste
status: Open
date: 2026-11-18
trigger: A 4c dar trabalho ao `init` (os handles pelo canal de boot e o lançamento de outros processos), ou a fase 5
paid_date:
---

# DEBT 0087 — O `init` só roda no boot de teste

## O que falta

O kernel lança o `init` do initramfs só com `rustos.selftest=1`, e o carregador, a árvore de regiões e o
código de usuário ficam fora da imagem `release-small` (o compilador os descarta). O boot normal sobe o shell do
kernel e não executa nenhum programa de usuário, então a imagem que se entrega nunca roda o caminho do carregador.

## Por que foi aceito

Na 4b o `init` só confere o que o kernel deixou na pilha e sai: é um autoteste, e o
[guidelines 05](../guidelines/05_recursos_e_desempenho.md) manda o autoteste que roda tarefa para o modo de teste.
Rodá-lo no boot normal custou, medido num experimento: de 1 a 3 ms no aarch64 com HVF (o boot vai de 2 a 4–7 ms) e
cerca de 10 ms no x86_64 em TCG (o código frio do carregador e do primeiro retorno ao usuário), sem nada para o
usuário. A regra "o boot normal só leva o que custa menos de ~2 ms em TCG" não deixa.

## Gatilho

A 4c dar trabalho ao `init` (recebe os handles pelo canal de boot e lança outros processos), ou a fase 5.
A parte 1 da 4c cumpriu a primeira metade: o `init` recebe o canal de boot e escreve pelo console por
handle. Falta lançar outros processos (`process_create`, parte 3), que é o que muda o orçamento do boot.

## Como pagar

Chamar `init::run` do boot normal e deixá-lo vivo, sem esperar; tirar o `cfg(selftest)` de `mod init` e os
`allow(dead_code)` de `process.rs`, `loader.rs` e `vm.rs`; o `smoke-fast` passa a exigir `init loaded` e a saída
dele; o custo entra no `boot time:` e no [SDD 07](../sdd/07_desempenho_e_recursos.md). O dono decide o orçamento do
boot nessa hora.
