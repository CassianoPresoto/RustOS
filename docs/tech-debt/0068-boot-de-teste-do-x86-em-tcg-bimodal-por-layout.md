---
id: DEBT-0068
title: Tempo do boot de teste do x86_64 em TCG varia até 10x com o tamanho e o alinhamento do código
status: Open
date: 2026-11-18
trigger: O `make smoke` do x86_64 passar de 10 s, ou o CI do x86_64 estourar o tempo do smoke
paid_date:
---

# DEBT 0068 — Tempo do boot de teste do x86_64 em TCG varia até 10x com o tamanho e o alinhamento do código

## O que falta

As medidas curtas do boot de teste (`switch:`, `scale:`, `alloc storm:`) levam de 0,3 a 3,4 s no x86_64 em TCG conforme o binário:
a mesma lógica, com bytes de código a mais ou a menos, cai num modo rápido ou num lento. Medido na revisão do PR com
preenchimento de `nop`: a `main` com 2 500 a 5 000 bytes a mais fica lenta; este PR com 3 000 a mais fica rápido (83 a 88 ms de
`benches`, mais rápido que a `main`). Não é bimodal por boot: **cada binário fica quase sempre num modo** (nos boots deste PR, 1 em 5
saiu no outro). Nos boots lentos a troca de contexto na mesma CPU sai a 13–26 µs (contra 1,5), o `scale:` de 1 CPU a 340–460 ms (contra 93)
e o `alloc storm` a 147 ms (contra 25). Vale com `-accel tcg,thread=single` e com `thread=multi`; as estatísticas do JIT (`info jit`) são iguais.
O SDD 15 já registra o mesmo sintoma (dois modos por boot no `wake self-test:`, hipótese da TLB do TCG com o layout sorteado pelo KASLR).

A primeira hipótese deste PR (que uma chamada a `BootParams::parse("")` no `kmain` virava o binário para o modo lento) **não se reproduz** na
`main` atual: o que decide é o tamanho e o alinhamento do código, não essa chamada.

Efeito: o `make smoke` do x86_64 foi de cerca de 2,7 s para cerca de 4,5 s de parede neste PR, sem trabalho novo (o `self-tests` de 340 ms e o
resto do boot são iguais).

## Por que foi aceito

Só o `make smoke` roda isso, e TCG não serve para número de latência (guidelines 05). O boot normal, que é o que o usuário vê, não depende
disso (25 a 31 ms). Não se achou a causa.

## Gatilho

O `make smoke` do x86_64 passar de 10 s, ou o CI do x86_64 estourar o tempo do smoke.

## Como pagar

Achar o que muda com o tamanho e o alinhamento do código: o layout sorteado pelo KASLR contra a TLB do TCG, ou algo no escalonador que depende do
endereço (falso compartilhamento, [DEBT-0061](0061-percpu-sem-separar-linhas-de-cache.md)). Repetir o par rápido/lento com o preenchimento de `nop` e a medida de qual CPU roda cada
tarefa de `ping_pong`; se a causa for o TCG, fixar o alinhamento do que é quente.
