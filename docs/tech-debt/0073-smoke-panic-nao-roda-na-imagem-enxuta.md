---
id: DEBT-0073
title: O `smoke-panic` não roda na imagem enxuta (`release-small`) nem no `make pr`
status: Open
date: 2026-11-18
trigger: Um `cfg(feature = "selftest")` entrar em `panic.rs`, `early.rs` ou `console.rs`, ou o primeiro defeito de pânico que só a `release-small` mostre
paid_date:
---

# DEBT 0073 — O `smoke-panic` não roda na imagem enxuta nem no `make pr`

## O que falta

O `make pr` e o CI rodam o `smoke-panic` só no perfil pedido (`dev` ou `release`), que leva a feature `selftest`. O `smoke-fast` da `release-small` cobre a recusa do modo de teste e do `bench` ([SDD 06](../sdd/06_testes.md)), mas não o caminho de pânico e de exceção sem a feature. Rodado à mão, `make smoke-panic ARCH=all PROFILE=release-small` passa nos nove cenários das duas ISAs (2026-11-18).

## Por que foi aceito

O `smoke-panic` custa nove boots por ISA, cerca de 27 s no x86_64 e 54 s no aarch64. Hoje `panic.rs`, `early.rs` e `console.rs` não têm `cfg(feature = "selftest")`: o que a imagem enxuta perde é o `bench`, os autotestes e as medidas, que o pânico não toca. O `make clippy` já compila os quatro conjuntos de features nos dois alvos.

## Gatilho

O mesmo do frontmatter: um `cfg(feature = "selftest")` num módulo do caminho de pânico ou de exceção.

## Como pagar

Acrescentar `$(XTASK) smoke-panic --arch all --profile release-small` ao `make pr` e um passo ao job `smoke` do CI.
