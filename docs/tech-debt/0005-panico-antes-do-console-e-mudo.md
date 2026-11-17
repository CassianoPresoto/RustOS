---
id: DEBT-0005
title: Pânico antes do console, ou com o console preso, não aparece
status: Open
date: 2026-11-17
trigger: O primeiro boot que trave sem saída na serial, ou o primeiro boot em metal
paid_date:
---

# DEBT 0005 — Pânico antes do console, ou com o console preso, não aparece

## O que falta

O `kmain` lê o bootloader e monta a memória antes do console, porque no aarch64 a
serial precisa de frames para as tabelas de página. Um pânico nesses passos para a
CPU sem escrever nada: o `make smoke` só vê o tempo esgotar.

O mesmo acontece depois do console. `print_panic` e o relatório de exceção usam
`try_lock` no `SERIAL` e no `SCREEN`. Uma falha que aconteça com um deles preso
(um page fault dentro de `Sink::write_str` ou de `Screen::render`) para a CPU sem
imprimir.

## Por que foi aceito

Os dois passos falham só com bootloader quebrado ou mapa de memória vazio, e o
smoke passa nas duas ISAs. Um console precoce exige um caminho de saída sem
mapeamento em cada ISA.

## Gatilho

O primeiro boot que trave sem nenhuma linha `[rustos]` na serial, ou o primeiro
boot em metal (onde não há QEMU para inspecionar).

## Como pagar

No x86_64, iniciar a COM1 antes de tudo (não precisa de memória). No aarch64,
console precoce por semihosting na QEMU, ou pelo framebuffer que o Limine entrega.
