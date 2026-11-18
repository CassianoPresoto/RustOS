---
id: DEBT-0066
title: Calibração do TSC pelo PIT por latch, medida só em TCG
status: Open
date: 2026-11-18
trigger: O primeiro boot do x86_64 com KVM ou em metal
paid_date:
---

# DEBT 0066 — Calibração do TSC pelo PIT por latch, medida só em TCG

## O que falta

`kernel/src/platform/x86_64.rs` calibra o TSC e o contador do APIC com `pit::calibrate`, uma janela que fecha em 400 ppm, ou aos 5 ms se
mais tempo não chega ao alvo, com teto de 20 ms e recusa acima de 1% ([SDD 10](../sdd/10_interrupcoes.md#calibração-pelo-pit-uma-janela-que-cresce-até-fechar)).
Em TCG o erro medido contra o TSC exato de 1 GHz foi de até 0,094% (mediana 0,037%) em 12 boots. Em KVM ou em metal a latência das portas
do PIT é outra e o latch do canal 2 nunca rodou lá: ninguém mediu erro nem tempo.

## Por que foi aceito

Cortou o boot do x86_64 em TCG (20 ms fixos viraram 5), onde o relógio já é só para comparar o RustOS consigo mesmo. A frequência do TSC
não decide nenhuma correção hoje (o `timer:` aceita de 25 a 200 ms para 50).

## Gatilho

O primeiro boot do x86_64 com KVM ou em metal, onde o TSC deixa de ser o relógio virtual do TCG e
`now_ns` passa a ser número de latência.

## Como pagar

Repetir a medida do SDD 10 (quatro boots por janela) no hardware novo. Se o erro passar de ~0,05%, subir o teto da janela (o Linux vai a 50 ms) ou usar a transição do byte alto como o `pit_expect_msb` ou ler o `CPUID` 0x15 e 0x16 antes do PIT quando o hardware os der.
