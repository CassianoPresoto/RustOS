---
id: DEBT-0155
title: Etiquetas de TLB no x86_64 (PCID e INVPCID) desligadas e sem execução
status: Open
date: 2026-11-23
trigger: O primeiro boot em metal, ou em KVM, com uma CPU x86_64 que anuncie PCID
paid_date:
---

# DEBT 0155 — Etiquetas de TLB no x86_64 (PCID e INVPCID) desligadas e sem execução

## O que falta

O caminho com PCID do x86_64 (`enable_tags`, `load_tags_cpu`, `switch_space` com etiqueta, `INVPCID` tipos 1 e 2 e
o esvaziamento por `CR4.PGE`) está escrito, compilado e revisado, mas **nunca rodou**. A constante `PCID_ENABLED` em
`kernel/src/platform/x86_64.rs` é `false`: o x86_64 segue o caminho sem etiqueta. A QEMU TCG recusa PCID e INVPCID
(`TCG doesn't support requested feature`), e o host de desenvolvimento é um Mac sem KVM. A linha de boot sai
`pcid off (untested)` quando a CPU anuncia PCID, e `pcid off (absent)` quando não anuncia.

## Por que foi aceito

Ligar o caminho em qualquer máquina que anuncie PCID levaria código sem execução para metal. O aarch64 fica ligado
porque foi exercitado em HVF e em TCG. Manter o caminho compilado mantém o desenho da 4e para a próxima máquina.

## Gatilho

O primeiro boot em metal, ou em KVM, com uma CPU x86_64 que anuncie PCID. Nesse boot liga-se `PCID_ENABLED`, roda-se
`make smoke ARCH=x86_64` (com o autoteste `asid self-test`) e mede-se a troca com o `asid bench` contra `pcid off`.

## Como pagar

Trocar `PCID_ENABLED` para `true`, rodar o smoke e o autoteste de etiquetas, medir a troca nas duas configurações,
registrar os números no SDD 07 e trocar `off (untested)` por `on` no SDD 17. Se a máquina não tiver `INVPCID`,
conferir o caminho com `CR4.PGE`.

Ao ligar o PCID entra também o **teste de migração**, que não existe hoje: ler a página em A, escrever em B
depois de a página mudar de permissão ou de frame, e reler em A, com A tendo a tradução velha na TLB. Nenhum
autoteste cobre isso (o *shootdown* só prova a tradução velha com a parte remota desligada).

