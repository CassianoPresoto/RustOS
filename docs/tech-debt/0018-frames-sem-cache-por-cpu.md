---
id: DEBT-0018
title: Alocador de frames sem o cache por CPU
status: Open
date: 2026-11-17
trigger: O primeiro núcleo secundário subir, na fase 3
paid_date:
---

# DEBT 0018 — Alocador de frames sem o cache por CPU

## O que falta

O `FrameCache` de `core/mm` existe e está testado, mas o kernel não o liga: todo
frame passa pelo lock do buddy (`kernel/src/frames.rs`).

## Por que foi aceito

Há uma CPU só até a fase 3, então o lock do buddy não disputa com ninguém. O
cache esconderia frames fora do `free_frames` (o `mem` e o `frames:` do boot
mentiriam), pediria uma identidade de CPU que o kernel ainda não tem, e o único
chamador frequente de frame solto, o slab do heap, já guarda uma página livre por
classe. Frame solto fora do boot só aparece com o page fault de usuário (fase 4).

## Gatilho

O primeiro núcleo secundário subir, na fase 3.

## Como pagar

Um `FrameCaches<CPUS, CAP>` com `CPUS` fixo, cada cache acessado só com as interrupções
mascaradas na CPU dona, e `Frames` passando a servir o frame solto por ele
(`cache.through(&mut buddy)`). O `free_frames` precisa somar o que está nos caches.
Junto vale ler a [DEBT-0017](0017-dupla-liberacao-escondida-no-cache-de-frames.md).

Ao ligar o cache, a promessa de zerar só depois de conferir não vale para frame
solto: `FrameCache::verify_used` com `count == 1` devolve `Ok` sem conferir, porque
conferir no buddy tiraria o ganho do cache (de 1,7 para 11,7 ns no host). Decidir
junto: conferir só sob modo de depuração, ou aceitar e escrever o porquê.
