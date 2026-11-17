---
id: DEBT-0042
title: Troca de contexto com o EEVDF custa o dobro da do rodízio
status: Open
date: 2026-11-17
trigger: O primeiro benchmark da troca de contexto contra o Linux (lat_ctx) no mesmo hardware
paid_date:
---

# DEBT 0042 — Troca de contexto com o EEVDF custa o dobro da do rodízio

## O que falta

No aarch64 com HVF, o ping-pong de `unpark` e `park` foi de 41 ns (linha do boot) e 77 ns
(comando `bench`) com o rodízio para 99 a 110 ns e 131 a 164 ns com o `Composite`. O custo
novo é a política (`wake` mais `sleep` na árvore, 39 ns medidos à parte; `pick_next`, 35 a
58 ns) e três leituras do relógio por ida e volta, de 23 ns cada (`isb` mais `mrs` mais a
conversão para ns).

## Por que foi aceito

O que se ganha: prazo, preempção por acordar e fatia curta para o interativo. O
rodízio não faz nada disso, e o custo absoluto (cerca de 100 ns) ainda é pequeno contra os
1,2 a 2,2 µs publicados para o Linux em outro hardware ([SDD 07](../sdd/07_desempenho_e_recursos.md)).

## Gatilho

O primeiro benchmark da troca de contexto contra o Linux (`lat_ctx`) no mesmo hardware.

## Como pagar

Ler o relógio uma vez por entrada no kernel (IRQ ou chamada) e passá-lo adiante; ler o
`CNTVCT_EL0` sem o `isb` onde a ordem não importa; medir de novo `bench` e escrever os dois
números no SDD 07. O `schedule_at` já evita uma leitura por `park`.
