---
id: DEBT-0134
title: O `ps` mostra `user` como nome de toda tarefa de processo
status: Open
date: 2026-11-19
trigger: O primeiro `ps` com dois programas diferentes rodando ao mesmo tempo, ou o `ps` mostrar processos criados por `process_create`
paid_date:
---

# DEBT 0134 — O `ps` mostra `user` como nome de toda tarefa de processo

## O que falta

A tarefa de um processo se chama `user` (`Spawn::name` é `&'static str`), e o `ps` distingue os processos só pelo número na coluna `pid`. Com `run count &` e `run sysinfo` ao mesmo tempo, quem lê não sabe qual pid é qual programa sem ter guardado a saída do `run`.

## Por que foi aceito

O nome do arquivo vive no initramfs, que é `'static`, mas o `ps` e a tarefa guardam `&'static str` e o nome é um fatiamento de bytes de cpio sem garantia de UTF-8; guardar uma cópia por tarefa custa um buffer por slot. Nada precisa disso no M1.

## Gatilho

O `ps` com mais de um processo vivo ao mesmo tempo em uso real, ou o `ps` mostrar o filho de um `process_create` (o nome vem do pai, e a chamada já valida `name_ptr` e `name_len` sem guardar).

## Como pagar

Um nome curto (16 bytes) no `Slot` da tarefa, copiado do arquivo na `run_program`, e o `ps` o mostra no lugar de `user`.
