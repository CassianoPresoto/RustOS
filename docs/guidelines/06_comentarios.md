# Guidelines 06 — Comentários

> **O que este tema cobre:** o que um comentário no fonte pode dizer, quanto pode
> ocupar e como citar a documentação.

## O critério

| A pergunta que ele responde | Onde a resposta mora |
| --- | --- |
| **O que quebra se alguém mexer aqui?** | no código |
| **Por que escolhemos assim?** | no SDD ou no guidelines; no código, no máximo um `DEBT-NNNN` |
| **Como isso funciona?** | em nenhum lugar: o código já responde |

## Regras

- **Bloco contíguo de comentário: no máximo 4 linhas**, contando `//`, `///`, `//!`
  e `// SAFETY:`. Vale para teste também.
- **Documentação de item público é `///`; de crate ou módulo, `//!`** na primeira
  linha, dizendo o papel dele no hexágono.
- **`// SAFETY:` diz o invariante, não a operação.** "o guard só existe com o lock
  adquirido", não "desreferencia o ponteiro".
- **Cite a documentação como `DEBT-NNNN`, e nada mais.** Inline vale:
  `// PL011 da QEMU virt (DEBT-0002).`
- **Cite só o que existe em `docs/tech-debt/`.** Dívida paga sai junto com as
  referências a ela.
- **Não parafraseie a dívida citada** nem escreva histórico ("antes era X") no
  fonte: histórico é do commit.
- **Fato que sai do fonte não se perde:** vira dívida, parágrafo no SDD ou regra
  aqui, no mesmo PR.

## Como se verifica

`make comments-check` (`cargo xtask check-comments`) falha em bloco acima de 4
linhas e em `DEBT-NNNN` sem arquivo em `docs/tech-debt/`. `make docs-check` faz o
mesmo com as citações em Markdown.
