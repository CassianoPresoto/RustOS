---
id: DEBT-0066
title: Calibração do TSC pelo PIT com janela de 5 ms, medida só em TCG
status: Open
date: 2026-11-18
trigger: O primeiro boot do x86_64 com KVM ou em metal
paid_date:
---

# DEBT 0066 — Calibração do TSC pelo PIT com janela de 5 ms, medida só em TCG

## O que falta

`kernel/src/platform/x86_64.rs` calibra o TSC e o contador do APIC contra 5 ms do PIT
(`PIT_WINDOW_TICKS`), não os 20 ms de antes. Em TCG o erro visto foi de até 0,28% (contra 0,07%
com 20 ms; [SDD 10](../sdd/10_interrupcoes.md#calibração-pelo-pit-janela-de-5-ms)). Em KVM ou em
metal a leitura das portas do PIT custa outra coisa, e ninguém mediu o erro lá.

## Por que foi aceito

Cortou 15 ms dos 40 do boot normal do x86_64 em TCG, onde o relógio já é só para comparar o RustOS
consigo mesmo. A frequência do TSC não decide nenhuma correção hoje (o autoteste `timer:` aceita de 25 a 200 ms
para 50).

## Gatilho

O primeiro boot do x86_64 com KVM ou em metal, onde o TSC deixa de ser o relógio virtual do TCG e
`now_ns` passa a ser número de latência.

## Como pagar

Repetir a medida do SDD 10 (quatro boots por janela) no hardware novo. Se o erro passar de ~0,05%, voltar a
janela a 20 ms só fora do TCG, ou ler o `CPUID` 0x15 e 0x16 antes do PIT quando o hardware os der.
