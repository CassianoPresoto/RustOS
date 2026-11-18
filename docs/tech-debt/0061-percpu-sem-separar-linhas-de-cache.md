---
id: DEBT-0061
title: PerCpu sem separar as linhas de cache por quem escreve
status: Open
date: 2026-11-17
trigger: O primeiro boot em metal, ou em VM com mais de 8 núcleos, em que a troca entre CPUs ou a fila por CPU meça pior que no HVF
paid_date:
---

# DEBT 0061 — PerCpu sem separar as linhas de cache por quem escreve

## O que falta

No `PerCpu` ([SDD 15](../sdd/15_tarefas_e_preempcao.md)) os campos que o dono escreve a cada troca (`locks_held`, `in_irq`, `current`, os contadores) dividem linha de cache com os que outras CPUs leem ou escrevem (`load`, `armed_ns`, `kick_pending`); e o lock da fila, que outras CPUs travam, divide a linha com o do cache de frames, que só a dona trava. Cada escrita remota em `load` ou no lock da fila invalida a linha que o dono usa.

## Por que foi aceito

Separar em linhas próprias (`Shared`, a fila e o cache de frames cada um com `#[repr(align(64))]`) foi implementado e medido no HVF, `release`, `-smp 4`, 4 rodadas antes e depois: troca na mesma CPU 64–76 ns contra 72–77, entre CPUs 5 292–5 631 contra 5 417–5 587, IPI de ida e volta 10 046–11 420 contra 10 607–11 284, tempestade com cache 107–197 µs contra 97–176. Sem ganho, e o bloco cresce de cerca de 500 para mais de 700 bytes por CPU. Nenhuma máquina com tráfego de linha entre núcleos real (metal, mais de 8 CPUs) foi medida.

## Gatilho

O primeiro boot em metal, ou em VM com mais de 8 núcleos, em que a troca entre CPUs ou o `bench` meça pior que no HVF.

## Como pagar

Agrupar os campos como acima, sem criar campo novo: `Shared` com `armed_ns`, `load`, `kick_pending` e `next_balance_ns`; `Aligned<IrqLock<Queue>>` e `Aligned<IrqLock<Cache>>`. Medir a troca entre CPUs, o IPI de ida e volta e a tempestade antes e depois, na mesma máquina.
