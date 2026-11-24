---
id: DEBT-0158
title: Ida e volta por canal entre dois processos 20 a 30 vezes acima do `lat_ctx`, sem decomposição
status: Open
date: 2026-11-24
trigger: O primeiro programa de `user/` que troque mensagem por canal a cada quadro ou a cada evento (o M2, se a entrada ou a tela passarem por canal), ou a primeira comparação publicada de troca entre processos com o Linux
paid_date:
---

# DEBT 0158 — Ida e volta por canal entre dois processos 20 a 30 vezes acima do `lat_ctx`, sem decomposição

## O que falta

A medida da 4f (`[user] procs: channel ping-pong`) dá 42,1 µs por ida e volta no aarch64 em HVF
(38,5 a 44,0 µs, 5 boots), contra 1,2 a 2,2 µs do `lat_ctx` de referência
([SDD 17, "Medidas da fase"](../sdd/17_processos_isolamento_e_syscalls.md#medidas-da-fase)), enquanto a
syscall nula custa 70 ns e a troca entre espaços, 58 ns. O custo está em outro lugar do caminho
(acordar a tarefa da outra ponta, IPI, fila, espera até o tique): ninguém decompôs.
Na mesma rodada, as faltas de página do aarch64 (958 ns no primeiro toque) saíram acima das da 4c
(0,51 a 0,69 µs), e o menor processo ficou em 13 a 14 frames contra 14 a 15: também sem investigar.

## Por que foi aceito

A 4f fecha a fase com o número medido e honesto; decompor o acordar entre CPUs é trabalho de
escalonador (SDD 14 e 15) e não muda contrato. Nenhum programa de hoje troca mensagem em laço.

## Gatilho

O primeiro programa de `user/` que troque mensagem por canal a cada quadro ou a cada evento (o M2,
se a entrada ou a tela passarem por canal), ou a primeira comparação publicada de troca entre
processos com o Linux.

## Como pagar

Medir as partes do caminho (`channel_write` até o `unpark`, o IPI de reescalonamento, a fila até a
outra ponta rodar) com o relógio do kernel, as duas pontas pinadas na mesma CPU e em CPUs
diferentes; corrigir o que dominar e remedir com o mesmo comando. Rever, no mesmo PR, a diferença
das faltas de página contra a 4c.
