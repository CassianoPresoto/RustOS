---
id: DEBT-0050
title: Histórico de rolagem desligado no aarch64
status: Open
date: 2026-11-17
trigger: O primeiro teclado no aarch64 (virtio-input, fase 8) ou um atalho de rolagem pela serial
paid_date:
---

# DEBT 0050 — Histórico de rolagem desligado no aarch64

## O que falta

O kernel só aloca o histórico do terminal quando `platform::has_keyboard()` é
verdadeiro. A `virt` do aarch64 não tem teclado (`keyboard_event` devolve `None`),
então a tela dela não guarda linhas que saíram do topo e nada rola para trás:
`screen: ..., history 0 lines` no boot. Quem lê a tela do aarch64 perde o que
passou (o log de boot, por exemplo) assim que ele rola.

## Por que foi aceito

Sem entrada de rolagem, 252 KiB (1 008 bytes por linha, 256 linhas) ficariam
alocados e inalcançáveis. A memória é o que o projeto mais quer gastar pouco.

## Gatilho

O primeiro teclado no aarch64 (virtio-input, fase 8) ou um atalho de rolagem pela
serial (uma sequência de teclas que o shell reconheça e entregue a
`scroll_view_back`).

## Como pagar

Ligar `console::enable_history` também nesse caso: com virtio-input, `has_keyboard()`
passa a ser verdadeiro e nada mais muda; com atalho pela serial, o shell chama
`scroll_view_back` e `scroll_view_forward` por uma porta nova e a condição de
`kernel/src/main.rs` inclui a serial.
