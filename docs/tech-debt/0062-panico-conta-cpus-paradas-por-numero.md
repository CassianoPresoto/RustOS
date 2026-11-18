---
id: DEBT-0062
title: O pânico conta as CPUs paradas por número, não por máscara
status: Open
date: 2026-11-18
trigger: O primeiro pânico durante a subida dos núcleos que saia com texto de outra CPU intercalado, ou o primeiro boot em metal com mais de 4 núcleos
paid_date:
---

# DEBT 0062 — O pânico conta as CPUs paradas por número, não por máscara

## O que falta

O dono do pânico (`kernel/src/panic.rs`) lê `online_count()` para saber quantas CPUs
esperar e espera `STOPPED >= others`. Um núcleo que fica online logo depois dessa
leitura não entra na conta, mas, ao ver o pânico no giro do `GO`
(`panic::stop_if_requested`), para e soma em `STOPPED`. Esse incremento a mais pode
fechar a conta antes que um núcleo que estava nela pare de fato, e o dono segue para
`break_locks` um pouco cedo.

## Por que foi aceito

Só acontece num pânico durante a subida dos núcleos, e o efeito é o mesmo do caminho
de tempo esgotado que já existe: o dono toma o console mesmo assim. Achado na revisão
do PR do SMP; nenhum smoke o produz.

## Gatilho

O primeiro pânico durante a subida dos núcleos que saia com texto de outra CPU
intercalado, ou o primeiro boot em metal com mais de 4 núcleos.

## Como pagar

Trocar a contagem por máscaras: cada CPU acende o seu bit em `STOPPED_MASK`, e o dono
espera `STOPPED_MASK & ONLINE_MASK == ONLINE_MASK` sem o próprio bit, relendo a
`ONLINE_MASK` a cada volta.
