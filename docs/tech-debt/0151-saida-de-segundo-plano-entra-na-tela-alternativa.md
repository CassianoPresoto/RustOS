---
id: DEBT-0151
title: A saída de segundo plano entra na tela alternativa
status: Open
date: 2026-11-19
trigger: O primeiro programa de tela cheia (vi, less) da fase 5 rodando enquanto há processo em segundo plano que escreve
paid_date:
---

# DEBT 0151 — A saída de segundo plano entra na tela alternativa

## O que falta

Com o comando `alt` (tela alternativa ligada), o shell não mostra prompt, mas `userout` segue tratando o shell como estando no prompt: a saída de um processo em segundo plano apaga a "linha do prompt" que o shell supõe (a linha onde o cursor está) e depois redesenha prompt e linha em cima da tela alternativa.

## Por que foi aceito

Só o comando de demonstração `alt` liga a tela alternativa hoje, e ninguém roda um processo que escreve em segundo plano ao mesmo tempo. O shell não tem o conceito de "um programa é dono da tela".

## Gatilho

O primeiro programa de tela cheia (vi, less) da fase 5 rodando enquanto há processo em segundo plano que escreve, ou o job control.

## Como pagar

O `Shell` guardar que a tela alternativa está ligada (já vê o `?1049h` que ele mesmo envia, ou o programa passa a avisar) e `interject` deixar a fila parada, ou `userout` voltar ao estado direto, enquanto isso valer.
