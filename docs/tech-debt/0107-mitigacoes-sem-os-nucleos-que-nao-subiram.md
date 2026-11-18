---
id: DEBT-0107
title: A linha `mitigations:` não soma os núcleos que não subiram
status: Open
date: 2026-11-18
trigger: O primeiro boot em metal com mais de 8 núcleos, ou com um núcleo que falhe ao subir
paid_date:
---

# DEBT 0107 — A linha `mitigations:` não soma os núcleos que não subiram

## O que falta

A linha é a união (o pior de cada vulnerabilidade) do núcleo de boot e dos secundários que ficaram
online. Os que passam do limite de 8 (`MAX_CPUS`), os que falham ao subir e os que nunca respondem
não somam, e uma máquina heterogênea em que só eles precisam de uma mitigação sai com
`not needed`. O `cpu features:` do modo de teste é só do núcleo de boot.

## Por que foi aceito

Esses núcleos nunca rodam código do kernel (ficam parados), então a mitigação que precisam não
tem onde atuar. O Linux avalia cada CPU online, como aqui.

## Gatilho

O primeiro boot em metal com mais de 8 núcleos, ou com um núcleo que falhe ao subir.

## Como pagar

Se o kernel passar a rodar mais de 8 núcleos, nada muda (todos somam). Se um núcleo falhar,
imprimir um aviso com a máscara dos que ficaram de fora da união.
