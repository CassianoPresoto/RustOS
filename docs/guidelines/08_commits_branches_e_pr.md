# Guidelines 08 — Commits, branches e PR

> **O que este tema cobre:** o formato do commit, da branch e do PR, e o caminho de
> uma mudança até a `main`.

## Branches

- **Ninguém faz push direto na `main`**: tudo entra por PR, com o CI verde. A
  exceção foi o commit inicial do repositório.
- **Uma branch, um assunto:** `tipo/slug-em-ingles`, saindo da `main` atualizada
  (`feat/x86-interrupts`, `fix/pl011-fifo-wait`). A branch é apagada no merge.
- **Branch publicada recebe a `main` por merge**, sem rebase de histórico que já foi
  para o remoto.

## Commits

```text
tipo(escopo): descrição curta em português

O corpo, quando há, diz o porquê. O diff já mostra o quê.
```

| Tipo | Quando |
| --- | --- |
| `feat` | Capacidade nova do sistema |
| `fix` | Corrige comportamento errado |
| `perf` | Melhora medida de desempenho ou de memória, com o número no corpo |
| `refactor` | Muda estrutura sem mudar comportamento |
| `docs` | Só documentação |
| `test` | Só teste ou infraestrutura de teste |
| `build` | Build, toolchain, xtask |
| `ci` | Pipeline |
| `chore` | Manutenção que não é nenhum dos outros |

- **O escopo é a área:** `mm`, `sched`, `hal`, `arch-x86_64`, `arch-aarch64`,
  `boot`, `drivers`, `kernel`, `xtask`, `docs`, `ci`.
- **Descrição em português, curta, sem ponto final.** Um commit, um assunto.

## O PR

- **Template preenchido** (`.github/pull_request_template.md`): o quê, *Decisões*,
  o que foi rodado e o que não foi, dívidas.
- **`make pr` verde na máquina de quem abre** e o CI verde no GitHub.
- **Decisão nos seis casos do `AGENTS.md` abre em Draft** só com o diff dos docs.
- **Mudou desenho, regra ou número medido:** o diff do SDD ou do guidelines vem no
  mesmo PR.

## Como se verifica

```sh
git log --format=%s origin/main..HEAD | grep -vE '^(feat|fix|perf|refactor|docs|test|build|ci|chore)\([a-z0-9-]+\): '
```
