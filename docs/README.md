# Documentação

Três pastas, três perguntas. Cada uma tem `README.md` (as regras da pasta e o
índice) e `_template.md` (de onde todo arquivo novo nasce).

| Onde | Responde | Natureza |
| --- | --- | --- |
| [`sdd/`](sdd/README.md) | "O que é o projeto e como trabalho nele?" | Descritivo. Estado atual. |
| [`guidelines/`](guidelines/README.md) | "Como eu escrevo isto?" | Prescritivo. Regras verificáveis. |
| [`tech-debt/`](tech-debt/README.md) | "O que se sabe que falta?" | Dívidas conscientes, **com gatilho**. |

## Duas convenções de nome

| Pasta | Nome | O número é |
| --- | --- | --- |
| `tech-debt/` | `0001-titulo-curto.md` | **identidade**: nunca se reaproveita; o arquivo tem frontmatter (`id`, `status`, `trigger`) |
| `sdd/`, `guidelines/` | `01_tema.md` | **ordem de leitura**: capítulo pode ser fundido, renumerado ou apagado; sem frontmatter |

Dívida precisa ser referenciável (`DEBT-0001`) enquanto existe; documento vivo
muda de forma junto com o sistema.

## O fluxo

```text
tarefa → o agente decide pelo SDD e pelo guidelines → diff em sdd/ ou guidelines/ no PR
                                                         ↓
                                  seção Decisões do PR lista o que se decidiu
                                                         ↓
                                            o review aprova junto com o código
```

Não há documento de proposta. Nos seis casos em que o agente para (lista em
[`AGENTS.md`](../AGENTS.md#decisões)), o PR abre em Draft só com o diff dos docs, e
o merge dele é a aprovação.

Regras que valem nas três pastas:

- Arquivo novo nasce do `_template.md` da pasta.
- **SDD e guidelines são a fonte da verdade.** Se o código e o SDD divergirem, um
  dos dois está errado: corrija no mesmo PR.
- Dívida paga sai da pasta, junto com as referências a ela no código.
- Índices são manuais enquanto couberem numa tela.
- `make docs-check` confere o mecânico: índices, frontmatter das dívidas e links.
