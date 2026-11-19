---
id: DEBT-0087
title: O `init` não sobe sozinho no boot normal
status: Open
date: 2026-11-18
trigger: O dono fixar o orçamento do boot que cabe o `init` (com o filho), ou a fase 5
paid_date:
---

# DEBT 0087 — O `init` não sobe sozinho no boot normal

## O que falta

O kernel lança o `init` do initramfs só com `rustos.selftest=1`: o boot normal sobe o shell e nenhum processo. Desde o M1 o carregador, a árvore de regiões e o código de usuário **estão** na imagem `release-small`, e o shell lança qualquer programa do initramfs com `run` (inclusive `run init`, que roda as mesmas conferências e sai com 0). O que falta é o `init` ser o primeiro processo do boot normal e ficar vivo, lançando os outros.

## Por que foi aceito

O `init` ainda é um autoteste: confere o que o kernel deixou na pilha e sai. Rodá-lo no boot custou, medido num experimento da 4b, de 1 a 3 ms no aarch64 com HVF (o boot vai de 2 a 4–7 ms) e cerca de 10 ms no x86_64 em TCG, sem nada para o usuário, e a regra "o boot normal só leva o que custa menos de ~2 ms em TCG" ([guidelines 05](../guidelines/05_recursos_e_desempenho.md)) não deixa. O M1 não precisa dele: quem lança os programas é o shell, a pedido de quem digita.

## Gatilho

O dono fixar o orçamento do boot que cabe o `init` (a parte 3 da 4c o fez criar um filho: medido em `procs`, de 0,6 ms por processo no aarch64 em HVF a 9 ms no x86_64 em TCG, fora o carregamento do próprio `init`), ou a fase 5. Os gatilhos de antes (o canal de boot, o carregador na imagem de produção e o `process_create`) já foram cumpridos.

## Como pagar

Chamar `init::run` do boot normal e deixá-lo vivo, sem esperar; tirar o `cfg(selftest)` de `mod init`; o `smoke-fast` passa a exigir `init loaded` e a saída dele (e a sessão do shell deixa de ser a única prova do carregador no boot normal); o custo entra no `boot time:` e no [SDD 07](../sdd/07_desempenho_e_recursos.md). O dono decide o orçamento do boot nessa hora.
