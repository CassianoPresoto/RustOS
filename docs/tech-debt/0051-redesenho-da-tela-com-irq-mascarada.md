---
id: DEBT-0051
title: Redesenho da tela com a IRQ mascarada
status: Open
date: 2026-11-17
trigger: O primeiro benchmark de latência de IRQ ou de wakeup com a tela ligada, ou o primeiro código de tempo real que divida a CPU com o shell
paid_date:
---

# DEBT 0051 — Redesenho da tela com a IRQ mascarada

## O que falta

`console::flush` segura o `IrqLock` da tela enquanto `Screen::render` escreve no
framebuffer, então a IRQ fica mascarada durante o desenho. Rolar a visão
(Shift+PageUp) ou uma linha nova com a tela cheia suja a grade inteira: medido em
`release`, 27,7 ms no x86_64 (QEMU TCG) e 1,5 ms no aarch64 (HVF)
([SDD 07](../sdd/07_desempenho_e_recursos.md)). O desenho da rolagem já roda fora da
IRQ do teclado, na tarefa do shell, mas a máscara continua.

## Por que foi aceito

Hoje nada mede latência de IRQ com a tela em uso, e o número de TCG só compara o
RustOS consigo mesmo. Tirar o desenho de baixo do lock pede uma cópia das linhas
sujas ou um lock que não mascare, e nenhuma das duas vale antes de haver o que
medir.

## Gatilho

O primeiro benchmark de latência de IRQ ou de wakeup com a tela ligada, ou o
primeiro código de tempo real que divida a CPU com o shell.

## Como pagar

Copiar as linhas sujas (célula a célula, por linha) para um buffer curto com o lock,
soltar o lock e desenhar de lá; ou desenhar uma linha por vez, soltando o lock entre
elas. Medir de novo com o mesmo comando do SDD 07.
