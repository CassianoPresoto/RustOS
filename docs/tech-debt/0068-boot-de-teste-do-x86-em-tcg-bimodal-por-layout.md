---
id: DEBT-0068
title: Medidas do boot de teste do x86_64 em TCG mudam de 0,55 s para 1,3 a 2,5 s com uma mudança de layout
status: Open
date: 2026-11-18
trigger: Quando um número do `bench`, do `scale:` ou do `switch:` do x86_64 em TCG for citado em PR ou no SDD, ou o `make smoke` do x86_64 passar de 10 s
paid_date:
---

# DEBT 0068 — Medidas do boot de teste do x86_64 em TCG mudam de 0,55 s para 1,3 a 2,5 s com uma mudança de layout

## O que falta

As medidas curtas do boot de teste (`switch:`, `scale:`, `alloc storm:`) levam 0,55 s no x86_64 em TCG com um
binário e de 1,3 a 3,4 s com outro, **sem mudança de lógica**: acrescentar ao `kmain` uma chamada a
`BootParams::parse("")` basta, e tirá-la volta ao tempo curto (4 boots alternados de cada, `release`, mesma
máquina; o `core::hint::black_box` de uma constante não muda nada). O binário lento também pode dar um boot curto
(1 em 5, 707 ms no total). Nos boots lentos a troca de contexto na mesma CPU sai a 13–26 µs (contra 1,5), o `scale:` de 1 CPU a 340–460 ms (contra 93) e o
`alloc storm` a 147 ms (contra 25). Vale com `-accel tcg,thread=single` e com `thread=multi` (os dois tempos curto e longo se repetem nos dois) e, com `-smp 1`, o mesmo binário lento
também oscilou (1,6 e 13 µs por troca). As estatísticas do JIT (`info jit`: TBs, invalidações) são iguais nos dois.

## Por que foi aceito

Só o `make smoke` roda isso, e TCG não serve para número de latência (guidelines 05). O boot normal, que é o que o
usuário vê, não depende disso (24 ms, estável). Não se achou a causa: a QEMU 11.1.3 roda os vCPUs do x86_64
em threads do hospedeiro ([SDD 05](../sdd/05_build_e_execucao.md)), e o tempo muda com o layout do binário.

## Gatilho

Quando um número do `bench`, do `scale:` ou do `switch:` do x86_64 em TCG for citado em PR ou no SDD, ou o `make smoke` do x86_64 passar de 10 s.

## Como pagar

Achar o que muda com o layout: afinidade dos vCPUs no hospedeiro, ou algo no escalonador que depende do endereço (falso
compartilhamento de linha, [DEBT-0061](0061-percpu-sem-separar-linhas-de-cache.md)). Repetir o par rápido/lento com `-accel tcg,thread=multi`, `-smp 1` e a
medida de qual CPU roda cada tarefa de `ping_pong`.
