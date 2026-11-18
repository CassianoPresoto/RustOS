---
id: DEBT-0063
title: Acordar com IrqLock preso, ou com a IRQ mascarada pelo chamador, só troca na próxima IRQ
status: Open
date: 2026-11-17
trigger: O primeiro chamador de `unpark` ou `spawn` que segure um `IrqLock` ou mascare a IRQ e dependa da troca na hora
paid_date:
---

# DEBT 0063 — Acordar com `IrqLock` preso, ou com a IRQ mascarada pelo chamador, só troca na próxima IRQ

## O que falta

`unpark` e `spawn` trocam ao fim da chamada quando a acordada deve preemptar (`preempt_now`, [SDD 15](../sdd/15_tarefas_e_preempcao.md),
"Preempção"). Não trocam se o chamador já tinha a interrupção mascarada ou segura um `IrqLock`: o pedido fica na
política e vale no fim da próxima IRQ. O Linux adia para o ponto em que o contador de preempção zera (`preempt_enable`);
aqui o adiamento é até a IRQ seguinte, no pior caso o fim da fatia da corrente.

## Por que foi aceito

Nenhum chamador do kernel acorda com `IrqLock` preso: `notify_input` roda em IRQ (onde o `preempt` do fim dela cuida) e
as tarefas do `bench` não seguram lock. Um ponto de saída no `IrqGuard::drop` entraria no caminho de todo lock (heap,
frames, fila) sem caso que o peça.

## Gatilho

O primeiro chamador de `unpark` ou `spawn` que segure um `IrqLock`, ou que mascare a IRQ de propósito, e dependa de a
acordada rodar antes de ele seguir.

## Como pagar

Um bit `need_resched` por CPU que `preempt_now` liga quando recusa a troca, e que `IrqGuard::drop`
confere quando o último lock solta (`locks_held() == 0`) e a IRQ estava ligada: então `schedule`. Medir o custo da
conferência em todo `drop` de lock com o `bench` antes de entrar.
