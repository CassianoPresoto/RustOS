---
id: DEBT-0057
title: TSC e contadores entre núcleos supostos sincronizados
status: Open
date: 2026-11-17
trigger: O primeiro boot do x86_64 em metal ou com KVM (o mesmo de DEBT-0034)
paid_date:
---

# DEBT 0057 — TSC e contadores entre núcleos supostos sincronizados

## O que falta

O relógio (`now_ns`) é o TSC do x86_64, com a origem e a frequência medidas uma vez na CPU de boot contra o PIT e usadas por todos os núcleos; o timer do Local APIC também é calibrado só ali. No aarch64 o `CNTVCT_EL0` é do sistema e não tem esse problema. Se o TSC de um núcleo estiver defasado, o prazo `now_ns` de uma tarefa que migra anda para trás ou para a frente.

## Por que foi aceito

O TCG e o KVM entregam TSC comum a todas as vCPUs, e o relógio calibrado atende o que se mede hoje. Em metal a leitura do TSC por núcleo e o ajuste (`IA32_TSC_ADJUST`) não foram lidos.

## Gatilho

O primeiro boot do x86_64 em metal ou com KVM (o mesmo de [DEBT-0034](0034-tsc-deadline-sem-teste-no-x86.md)).

## Como pagar

Medir a defasagem do TSC de cada núcleo contra o da CPU de boot no `smp::start` (troca de leituras num ponto de encontro) e ou recusar o x86_64 sem TSC invariante e sincronizado ou guardar um deslocamento por CPU.
