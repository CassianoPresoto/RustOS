---
id: DEBT-0150
title: A fila de saída de segundo plano tem 1 KiB e descarta ao encher
status: Open
date: 2026-11-19
trigger: O primeiro programa em segundo plano que imprima mais de 1 KiB enquanto um comando do shell roda, ou o job control da fase 5
paid_date:
---

# DEBT 0150 — A fila de saída de segundo plano tem 1 KiB e descarta ao encher

## O que falta

Enquanto o shell escreve (um `ps`, um `bench`), a saída de usuário espera em `userout` numa fila de 1 024 bytes (cerca de doze linhas de 80 colunas). Uma escrita que não cabe é descartada inteira e o shell, ao esvaziar a fila, diz `[rustos] user output: N writes lost (backlog full)`. O sintoma é esse aviso na tela.

## Por que foi aceito

Nenhum programa da demonstração imprime mais que uma linha a cada meio segundo, e comandos do shell acabam em milissegundos (o `bench`, em segundos, só existe no modo de teste). Mil bytes em `.bss` custam menos que qualquer política de crescimento, e descartar com aviso é melhor que bloquear o processo escritor com o shell ocupado.

## Gatilho

O primeiro programa em segundo plano que escreva mais de 1 KiB durante um comando do shell, ou o job control da fase 5, que traz o primeiro plano e o segundo como ideia do usuário.

## Como pagar

Bloquear o escritor quando a fila enche (o `debug_write` espera o laço do shell esvaziá-la, fora da IRQ), ou dar a cada processo a sua própria fila limitada e intercalar por linha; a primeira é a menor mudança em `kernel/src/userout.rs`.
