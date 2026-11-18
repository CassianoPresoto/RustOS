---
id: DEBT-0055
title: Kernel sem TLB shootdown entre núcleos
status: Open
date: 2026-11-17
trigger: O primeiro mapeamento do kernel que seja removido ou tenha permissão alterada depois do boot (pilha liberada, `munmap`, W^X dinâmico)
paid_date:
---

# DEBT 0055 — Kernel sem TLB shootdown entre núcleos

## O que falta

Todos os núcleos usam as mesmas tabelas do kernel, mas `entry_changed` (`invlpg` no x86_64, `dsb` e `isb` no aarch64) só invalida o TLB da CPU que mexeu. Hoje isso basta: depois do boot o kernel só **acrescenta** mapeamentos (pilhas novas na janela de pilhas, dispositivos na janela de dispositivos), e entrada que passa de ausente para presente não precisa de invalidação em outra CPU. Nenhum mapeamento é removido nem muda de permissão.

## Por que foi aceito

Nenhum mapeamento do kernel é removido ou alterado depois do boot: a janela de pilhas é só de avanço ([SDD 13](../sdd/13_memoria_virtual.md)), e a pilha de tarefa que sai fica com o slot.

## Gatilho

O primeiro mapeamento do kernel removido ou com permissão alterada depois do boot, ou o primeiro espaço de usuário compartilhado entre núcleos (fase 4).

## Como pagar

IPI de invalidação (`invlpg` remoto no x86_64; `tlbi vae1is`, que já é do domínio *inner shareable*, no aarch64) e confirmação de todos os núcleos online antes de reusar o frame ou a página virtual. Testar com duas CPUs lendo a página que uma terceira remove.
