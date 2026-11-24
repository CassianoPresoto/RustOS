---
id: DEBT-0158
title: Ida e volta por canal entre dois processos 20 a 35 vezes acima do `lat_ctx`: o acordar por IPI domina
status: Open
date: 2026-11-24
trigger: O primeiro programa de `user/` que troque mensagem por canal a cada quadro ou a cada evento (o M2, se a entrada ou a tela passarem por canal), ou a primeira comparação publicada de troca entre processos com o Linux
paid_date:
---

# DEBT 0158 — Ida e volta por canal entre dois processos 20 a 35 vezes acima do `lat_ctx`: o acordar por IPI domina

## O que falta

A medida da 4f (`[user] procs: channel ping-pong`, `PROFILE=release`) dá 49,7 µs por ida e volta no
aarch64 em HVF (39,7 a 70,1 µs, 5 boots; 25 a 99 µs em 9 boots de três commits), contra 1,2 a 2,2 µs
do `lat_ctx` de referência ([SDD 07](../sdd/07_desempenho_e_recursos.md#fase-4-as-medidas-fatia-4f)).
As linhas do kernel nos mesmos boots apontam o custo: a troca na mesma CPU custa cerca de 42 ns, a
troca entre CPUs de 8 a 20 µs e a ida e volta de IPI de 9 a 27 µs, e a ida e volta por canal no mesmo
processo, de 1,5 a 2,5 µs. Cada ida e volta entre processos acorda duas vezes uma tarefa em outra CPU
(ociosa, em `wfi`), e esse acordar domina. Falta separar o que é do RustOS (o caminho até o IPI, a CPU
que dorme) do que é do HVF (a entrega do IPI virtual), e decidir se a tarefa acordada deve rodar na
CPU de quem a acordou.

## Por que foi aceito

A 4f fecha a fase com o número medido e honesto; decompor o acordar entre CPUs é trabalho de
escalonador (SDD 14 e 15) e não muda contrato. Nenhum programa de hoje troca mensagem em laço.

## Gatilho

O primeiro programa de `user/` que troque mensagem por canal a cada quadro ou a cada evento (o M2,
se a entrada ou a tela passarem por canal), ou a primeira comparação publicada de troca entre
processos com o Linux.

## Como pagar

Medir a ida e volta com as duas pontas pinadas na mesma CPU e em CPUs diferentes, e o caminho do
`channel_write` até a outra ponta rodar, com o relógio do kernel; comparar com o KVM ou o metal
quando houver; corrigir o que for do RustOS (por exemplo, acordar na CPU de quem acorda) e remedir com
o mesmo comando.
