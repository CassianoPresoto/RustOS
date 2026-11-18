---
id: DEBT-0042
title: Troca de contexto com o EEVDF gasta metade do tempo na política
status: Open
date: 2026-11-17
trigger: O primeiro benchmark da troca de contexto contra o Linux (lat_ctx) no mesmo hardware
paid_date:
---

# DEBT 0042 — Troca de contexto com o EEVDF gasta metade do tempo na política

## O que falta

A dívida nasceu com a troca do `Composite` (FIFO e EEVDF) no dobro da do rodízio. A parte do relógio
e dos locks foi paga: no aarch64 com HVF, 5 boots, a linha `switch:` do boot foi de 68 a 75 ns (mediana
72) para **41 a 44** (mediana 41), e o comando `bench` de 73 a 158 (mediana 132) para **42**. O que tirou o custo foi o
relógio do escalonador sem o `isb` (-23 ns), um lock só da decisão de bloquear à escolha (-2) e a conversão por
multiplicação e deslocamento (-4) ([SDD 07](../sdd/07_desempenho_e_recursos.md), "Otimização da troca e do acordar").

Sobram 41 ns, dos quais cerca de 18 são da política (`wake`, `sleep` e `pick_next` em treap, com `i128`),
11 dos dois locks da fila e o resto do `switch`. O rodízio faz o mesmo na política em cerca de 6 ns (host), então a
razão ainda não é 1, mesmo com o relógio igual. No x86_64 em TCG a política é metade da troca (680 de 1 300 ns), mas
é número de TCG ([DEBT-0041](0041-escalonador-do-x86-sem-medida-fora-do-tcg.md)).

## Por que foi aceito

O que se ganha: prazo, preempção por acordar e fatia curta para o interativo. O rodízio não faz nada disso, e o
custo absoluto (41 ns) já é pequeno contra os 1,2 a 2,2 µs publicados para o Linux em outro hardware.

## Gatilho

O primeiro benchmark da troca de contexto contra o Linux (`lat_ctx`) no mesmo hardware.

## Como pagar

Medir `lat_ctx` no mesmo hardware. Se a política ainda pesar, o que se conhece e não foi tentado: pesos por multiplicação
pelo inverso (`inv_weight` do `fair.c`) no lugar da divisão em `real_to_virtual`, e tirar o treap do caminho quando só uma
tarefa espera. Tentadas e sem ganho no host e no HVF (SDD 07): o atalho de `real_to_virtual` para o peso 1024 e o
`recenter` sem divisão. Só vale mexer com uma medida nova que aponte a peça.
