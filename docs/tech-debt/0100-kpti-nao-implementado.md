---
id: DEBT-0100
title: KPTI não implementado, e `kpti=on|off|auto` só entra no relatório
status: Open
date: 2026-11-18
trigger: O primeiro boot em metal numa CPU que a detecção marca com Meltdown
paid_date:
---

# DEBT 0100 — KPTI não implementado, e `kpti=on|off|auto` só entra no relatório

## O que falta

A linha `mitigations:` diz `kpti off (not implemented, would be on)` na CPU que a detecção marca
com Meltdown (x86_64: sem `RDCL_NO` e fora das listas da AMD e da Hygon; aarch64: sem `CSV3` e fora
da lista de CPUs imunes do Linux). O kernel mapeia a metade do kernel em toda raiz de usuário.
A chave `kpti=on|off|auto` da linha de comando já é lida e só muda o texto da linha.

## Por que foi aceito

A parte 1 da 4e entregou a detecção, o relatório e o clamp do Spectre v1; a parte 2, o PCID/ASID.
O dono decidiu (2026-11-24) não fazer o KPTI na fase 4: ele só protege CPU vulnerável a Meltdown
(Intel anterior a 2019), as máquinas de comparação de desempenho não são, e o Linux o desliga nelas.
O desenho segue no [SDD 17](../sdd/17_processos_isolamento_e_syscalls.md#o-trampolim-e-o-caminho-pronto-para-kpti).

## Gatilho

O primeiro boot em metal numa CPU que a detecção marca com Meltdown: ali a linha `mitigations:`
diria `kpti off (not implemented, would be on)` com o processo podendo ler o kernel.

## Como pagar

O trampolim e as duas raízes por processo do SDD 17; trocar `Enabled::kpti` por `Switch::On` (ou
`Untested`, se o metal for o único que prova) em `kernel/src/mitigation.rs`; medir a syscall nula com
`kpti=on` e `kpti=off`.
