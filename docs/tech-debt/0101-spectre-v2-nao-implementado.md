---
id: DEBT-0101
title: Spectre v2 sem mitigação no kernel (eIBRS, retpoline, IBPB, STIBP)
status: Open
date: 2026-11-18
trigger: O primeiro boot em metal
paid_date:
---

# DEBT 0101 — Spectre v2 sem mitigação no kernel (eIBRS, retpoline, IBPB, STIBP)

## O que falta

A linha `mitigations:` diz `v2 off (not implemented)` onde a CPU tem Spectre v2: no x86_64, toda
CPU fora da lista do Linux (a QEMU em TCG e a maioria das reais); no aarch64, a sem `CSV2` e fora
da lista de CPUs imunes. O kernel não liga `IBRS` (nem `AutoIBRS`) em `IA32_SPEC_CTRL`, não emite
`IBPB` na troca entre processos nem liga `STIBP`; no aarch64 não faz o endurecimento do preditor.

## Por que foi aceito

A QEMU em TCG não especula e o HVF e o KVM não expõem os controles como o metal: nada aqui se
exercita fora de hardware. O SDD 17 manda registrar com este gatilho o que não se testa.

## Gatilho

O primeiro boot em metal: a linha `mitigations:` daquela máquina decide o que ligar.

## Como pagar

No x86_64, `IBRS` uma vez no boot onde há eIBRS (`enhanced_ibrs()`), `IBPB` condicional na troca de
espaço e `STIBP` com SMT. O retpoline é mudança de ferramenta de build (caso 2 do `AGENTS.md`): no `rustc` 1.100.0 estável, `-C target-feature=+retpoline-indirect-calls` (e `-indirect-branches`, `-external-thunk`) só gera um aviso de que a feature "cannot be enabled with `-Ctarget-feature`: use `-Zretpoline` compiler flag instead" e vai virar erro (rust-lang/rust#162235), e `-Z` só é aceito no nightly ("the option `Z` is only accepted on the nightly compiler"); o RustOS não usa nightly (SDD 05). Onde há eIBRS não precisa dele. Cada um vira
`Switch::On` ou `Untested` em `kernel/src/mitigation.rs`, com o custo no SDD 07.
