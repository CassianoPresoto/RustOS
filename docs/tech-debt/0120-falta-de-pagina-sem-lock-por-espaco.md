---
id: DEBT-0120
title: Falta de página e troca de folha sem lock por espaço de usuário
status: Open
date: 2026-11-18
trigger: O primeiro `thread_create`, com duas threads de um processo em CPUs diferentes
paid_date:
---

# DEBT 0120 — Falta de página e troca de folha sem lock por espaço de usuário

## O que falta

`UserSpace::resolve_fault`, `unmap_page` e o `remap_page` por baixo leem a folha, decidem e gravam sem lock: pressupõem uma thread só por espaço, que é a única que falta, copia ou remove. O `Process` é lido por ponteiro cru a partir da tarefa (`resolve_user_fault`). Uma falta que resolve como `Retry` (a página já serve) volta sem contar: com uma thread ela só acontece por defeito do núcleo, e o laço de faltas no mesmo endereço não é detectado (nem no caminho da cópia, `on_copy_fault`).

## Por que foi aceito

Cada processo tem uma thread até a 4c entregar `thread_create`; um lock que nada disputa custaria um acesso atômico por falta sem proteger caso algum. O *shootdown* já espera as outras CPUs do espaço.

## Gatilho

O primeiro `thread_create`, com duas threads de um processo em CPUs diferentes.

## Como pagar

Um lock por espaço (`IrqLock`) que cobre da leitura da folha até o fim do *shootdown*, reconferindo a folha sob ele (o `pte_same` do Linux em `wp_page_copy`); `Retry` do `plan_fault` já é o caminho de quem perde a corrida. Contar `Retry` seguidos no mesmo endereço (o segundo é falha) para o laço não prender a CPU. Medir a falta com e sem o lock (SDD 07).
