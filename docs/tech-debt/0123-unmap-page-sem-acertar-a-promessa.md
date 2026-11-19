---
id: DEBT-0123
title: `unmap_page` não acerta a promessa de páginas da região preguiçosa
status: Open
date: 2026-11-19
trigger: O primeiro `vmar_unmap` (ou `vmar_protect`) sobre região preguiçosa
paid_date:
---

# DEBT 0123 — `unmap_page` não acerta a promessa de páginas da região preguiçosa

## O que falta

`UserSpace::unmap_page` remove a folha, invalida o TLB das CPUs do espaço e devolve o frame, mas não devolve a promessa da página: o contador `lazy_backed` e a reserva global só descem na morte do espaço. Se uma página própria de região preguiçosa é removida e a falta volta a criá-la no mesmo endereço, ela devolve a promessa uma segunda vez e o `commit::release` pode devolver mais do que foi prometido (o `debug_assert` de `kernel/src/commit.rs`). Hoje só o autoteste chama `unmap_page`; o anel de *shootdown* reserva uma página a cada cópia por isso.

## Por que foi aceito

Nenhuma syscall remove página: o `vmar_unmap` ainda não existe, e a árvore de regiões é quem sabe se a região é preguiçosa e se a página era própria.

## Gatilho

O primeiro `vmar_unmap` ou `vmar_protect` que alcance região preguiçosa.

## Como pagar

`unmap_page` recebe a região (ou o `vmar_unmap` a decide) e, se ela é preguiçosa e a folha era um frame próprio, desce `lazy_backed` e devolve ao contador a promessa de uma página, para a próxima falta prometer de novo. Testar remover e tocar a mesma página em laço sem passar da promessa.
