---
id: DEBT-0066
title: Calibração do TSC pelo PIT, adaptativa de 5 a 20 ms, medida só em TCG
status: Open
date: 2026-11-18
trigger: O primeiro boot do x86_64 com KVM ou em metal
paid_date:
---

# DEBT 0066 — Calibração do TSC pelo PIT, adaptativa de 5 a 20 ms, medida só em TCG

## O que falta

`kernel/src/platform/x86_64.rs` calibra o TSC e o contador do APIC com `pit::calibrate`: 5 ms de PIT, estendidos até 20 ms se a
incerteza estimada passar de 500 ppm ([SDD 10](../sdd/10_interrupcoes.md#calibração-pelo-pit-janela-adaptativa-de-5-a-20-ms)).
Em TCG o erro medido contra o TSC exato de 1 GHz foi de até 0,144% (mediana 0,017%) em 12 boots. Em KVM ou
em metal a latência das portas do PIT é outra, e ninguém mediu erro nem tempo lá; a incerteza devolvida é uma estimativa e já foi
excedida uma vez em TCG.

## Por que foi aceito

Cortou o boot do x86_64 em TCG (20 ms fixos viraram 5 a 20), onde o relógio já é só para comparar o RustOS consigo mesmo. A frequência do TSC
não decide nenhuma correção hoje (o `timer:` aceita de 25 a 200 ms para 50).

## Gatilho

O primeiro boot do x86_64 com KVM ou em metal, onde o TSC deixa de ser o relógio virtual do TCG e
`now_ns` passa a ser número de latência.

## Como pagar

Repetir a medida do SDD 10 (quatro boots por janela) no hardware novo. Se o erro passar de ~0,05%, subir o teto da janela (o Linux vai a 50 ms) ou ler o `CPUID` 0x15 e 0x16 antes do PIT quando o hardware os der.
