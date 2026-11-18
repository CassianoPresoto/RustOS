---
id: DEBT-0074
title: O validador de ELF recusa EI_OSABI diferente de 0, e o rust-lld grava 3 com `#[used]`
status: Open
date: 2026-11-18
trigger: O primeiro programa de usuário (ou o `user/rt`) que precise de `#[used]`, ou o carregador da fase 5 receber ELF de terceiros
paid_date:
---

# DEBT 0074 — O validador de ELF recusa EI_OSABI diferente de 0, e o rust-lld grava 3 com `#[used]`

## O que falta

O `core/elf` exige `EI_OSABI == ELFOSABI_NONE` (0), como o [SDD 17](../sdd/17_processos_isolamento_e_syscalls.md) escreve. **Medido** (`rustc 1.100.0`, `rust-lld`, `x86_64-unknown-none`): o mesmo programa mínimo sai com `EI_OSABI = 0` sem `#[used]` e com `EI_OSABI = 3` (`ELFOSABI_GNU`) com `#[used]`, que emite uma seção com `SHF_GNU_RETAIN`. O validador devolve `BadOsAbi` nesse caso.

## Por que foi aceito

A regra é contrato de formato de executável (caso 6 do `AGENTS.md`) e não é do agente mudar. Nenhum programa de usuário existe ainda, e o Linux não confere `EI_OSABI`, então a regra é só mais estrita que o molde.

## Gatilho

O primeiro programa de usuário (ou o `user/rt`) que precise de `#[used]`, ou o carregador da fase 5 receber ELF de terceiros (que podem trazer OSABI 3).

## Como pagar

A decisão é do dono (caso 6): aceitar também `ELFOSABI_GNU` (3) no `core/elf`, ou manter a regra e proibir `#[used]` nos programas do RustOS. Em qualquer caso, atualizar a linha `EI_OSABI` do SDD 17 e o teste `real_elf_with_the_gnu_osabi_lld_writes_for_retained_statics_is_refused` em `core/elf/src/tests.rs`.
