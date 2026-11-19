---
id: DEBT-0132
title: A saída de um processo em segundo plano cola na linha do prompt
status: Open
date: 2026-11-19
trigger: O primeiro programa interativo em segundo plano, ou o controle de trabalhos (job control) da fase 5
paid_date:
---

# DEBT 0132 — A saída de um processo em segundo plano cola na linha do prompt

## O que falta

Com `run count &` o shell devolve o prompt e o processo segue escrevendo `[user] ...` pelo console. A primeira linha dele cai logo depois do `rustos ~ % ` (que não termina em `\n`), e a linha que o usuário digita no meio fica misturada com as do processo. O smoke só confere a última linha de `count`, que começa limpa.

## Por que foi aceito

O console do kernel escreve por mensagem e não sabe se o shell está no meio de uma linha; consertar pede o shell redesenhar o prompt quando chega saída que não é dele, ou o terminal ganhar a noção de linha do shell. Nenhum programa da fase 4 precisa.

## Gatilho

O primeiro programa em segundo plano que o usuário deva ler enquanto digita, ou o job control da fase 5.

## Como pagar

O kernel marca a saída de usuário como não vinda do shell; o laço do shell, ao ver a marca, apaga a linha do prompt (`\r\x1b[K`), deixa a saída passar e redesenha prompt e linha em edição (já sabe: `redraw`).
