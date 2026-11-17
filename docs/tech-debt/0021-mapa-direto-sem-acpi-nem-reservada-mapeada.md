---
id: DEBT-0021
title: Mapa direto sem as tabelas ACPI nem as regiões reservadas que o Limine mapeia
status: Open
date: 2026-11-17
trigger: O primeiro código que lê ACPI (MADT, MCFG) ou outra região reservada mapeada, na fase 6
paid_date:
---

# DEBT 0021 — Mapa direto sem ACPI nem reservada mapeada

## O que falta

O mapa direto do kernel cobre só `Usable` e `BootloaderReclaimable`. O Limine
(base revisão 4 e 5) também mapeia ACPI reclaimable, ACPI NVS e `RESERVED_MAPPED`
(SMBIOS, serviços de runtime da EFI). Nenhuma delas está mapeada nas tabelas novas.

## Por que foi aceito

Nada no kernel lê essas regiões ainda, e mapear memória que ninguém usa só aumenta
a superfície. A região `RESERVED_MAPPED` ainda vira `Reserved` em `region_kind`.

## Gatilho

O primeiro código que lê ACPI ou DTB (fase 6, DEBT-0002 e DEBT-0009), porque ele
passa a falhar com page fault ao seguir o ponteiro do RSDP.

## Como pagar

Mapear cada região que o leitor pedir em página de leitura, sem execução, em
`Vm::build`; ou, melhor, numa janela própria, como os dispositivos. Traduzir o tipo 8
do Limine para um `RegionKind` próprio.
