# RustOS — instruções para agentes

Fonte única de contexto para qualquer assistente de código. `CLAUDE.md`,
`GEMINI.md` e `.github/copilot-instructions.md` são symlinks para cá. Edite
**este** arquivo.

## O projeto em uma frase

Sistema operacional do zero, em Rust, com arquitetura hexagonal e foco em gastar
o mínimo de memória e CPU — para rodar de hardware pequeno a máquinas grandes e
ser medido contra Linux e Windows. Não é distro Linux nem usa kernel de terceiros.

## A documentação, inteira

| Onde | O quê |
| --- | --- |
| `docs/sdd/` | O projeto e **como se desenvolve nele**: arquitetura, mapa de código, boot, build, testes, desempenho, roadmap. Descritivo. |
| `docs/guidelines/` | **Como escrever**: estilo, nomenclatura, camadas, `unsafe`, recursos, comentários, testes, commits. Prescritivo. |
| `docs/tech-debt/` | O que se sabe que falta, **com gatilho**. |

**Leia `docs/sdd/` e `docs/guidelines/` antes de escrever código.** O SDD diz
onde as coisas ficam e como estão; o guidelines diz como escrevê-las. A regra que
separa os dois: **se descreve como é, é SDD; se manda fazer, é guidelines.**

Cada pasta tem `README.md` com as regras dela e `_template.md` de onde nasce
arquivo novo. `NNNN-slug.md` em `tech-debt/` (número é identidade, com
frontmatter); `NN_tema.md` em `sdd/` e `guidelines/` (número é ordem de leitura).

## Regras que não se violam

- **Dependência aponta para dentro.** `core/` não conhece `arch/`, `boot/` nem
  `drivers/`; adaptador não conhece outro adaptador. Só o `kernel/` liga tudo.
  `make layers-check` confere ([guidelines 03](docs/guidelines/03_camadas_e_dependencias.md)).
- **`unsafe` só onde o guidelines 04 deixa, com `// SAFETY:`.** `core/` e
  `drivers/` têm `#![forbid(unsafe_code)]`; `asm!` só em `arch/`.
- **Nenhum crate externo.** Só `core`/`alloc` e o que está no workspace. Exceção é
  decisão (abaixo), não conveniência.
- **Mudou o desenho → `docs/sdd/` muda no mesmo PR.** Decisão vira diff no SDD ou
  no guidelines e é listada na seção *Decisões* do PR.
- **Evidência antes de conclusão.** Spec, datasheet, header do protocolo, teste,
  log da serial ou número medido — nunca memória do modelo. Diga de onde veio.
- **Otimização sem número não entra.** Mede antes e depois, com o mesmo comando
  ([SDD 07](docs/sdd/07_desempenho_e_recursos.md)).
- **Não crie exceção silenciosa a uma regra escrita.** Ou é defeito, ou a regra
  muda pelo caminho de [Decisões](#decisões).
- **Ficou algo por fazer?** Vira item em `docs/tech-debt/`, com gatilho — o evento
  que obriga a pagar, não uma intenção.

## Decisões

**O agente decide** tudo o que segue o desenho do SDD e as regras do guidelines,
mesmo que o caso seja novo: onde um arquivo mora, nome, porta nova no `hal`,
adaptador novo de dispositivo já suportado, algoritmo dentro de um crate de
`core/`, teste. Escreve a decisão no SDD (ou no guidelines) e a lista no PR.

**O agente para** — e só nestes casos, a lista é fechada:

1. Crate novo em `core/`, ou dependência entre camadas que o guidelines 03 não permite.
2. Crate externo ou binário externo novo (bootloader, firmware, ferramenta de build).
3. Regra do guidelines que muda, sai ou ganha exceção.
4. ISA, plataforma ou protocolo de boot novo.
5. `unsafe` em `core/` ou `drivers/`, ou `asm!` fora de `arch/`.
6. Contrato que o mundo de fora enxerga: syscall, formato de executável, ABI
   compatível com Linux.

Parar é: abrir o PR em **Draft** só com o diff do SDD e do guidelines que a decisão
produziria, escrever a pergunta e as opções na seção *Decisões* e encerrar a
tarefa. O merge do Draft é a aprovação; o código vem depois. Fora desses seis
casos, parar é defeito.

## Comentário no código

Comentário responde **o que quebra se mexerem aqui**. O porquê mora no SDD ou no
guidelines; no fonte, no máximo um `DEBT-NNNN`. **No máximo 4 linhas por bloco
contíguo**, inclusive `///`, `//!` e `// SAFETY:`. `make comments-check` confere.
Regra inteira em [guidelines 06](docs/guidelines/06_comentarios.md).

## Como registrar uma dívida

Copie `docs/tech-debt/_template.md` para `docs/tech-debt/<NNNN>-<slug>.md` (o
maior número da pasta + 1), preencha o `trigger` com um **evento** e acrescente a
linha no índice do `README.md` da pasta. Atalho: `/tech-debt`.

## Commits, branches e PR

`tipo(escopo): descrição em português`, branch `tipo/slug-em-ingles` saindo da
`main`, PR para a `main` com o template preenchido e `make pr` verde. Regra
inteira em [guidelines 08](docs/guidelines/08_commits_branches_e_pr.md).

## Comandos

| O quê | Comando |
| --- | --- |
| Lista os alvos | `make help` |
| Todos os gates (fmt, clippy nos 3 alvos, camadas, comentários, docs) | `make check` |
| Testes de unidade no host | `make test` |
| Compila o kernel | `make build ARCH=x86_64\|aarch64 PROFILE=dev\|release\|release-small` |
| Sobe na QEMU com a serial no terminal | `make run ARCH=…` |
| Sobe, espera `[rustos] boot ok` e sai | `make smoke ARCH=…\|all` |
| O mesmo no boot normal, com o tempo do kernel até o prompt (em `PROFILE=release-small`, a imagem enxuta, sem o modo de teste) | `make smoke-fast ARCH=…\|all` |
| Tamanho das seções do kernel | `make size ARCH=… PROFILE=…` |
| Checklist antes do PR | `make pr` (`check` + `test` + smoke e smoke-fast nas duas ISAs, e o smoke-fast da `release-small`) |

**`make check` verde não é kernel verificado.** Ele não sobe a QEMU. Mexeu em
`kernel/`, `arch/`, `boot/` ou `drivers/`: rode `make smoke ARCH=all` e diga no PR
o que rodou e o que não rodou.

## Idioma

Prosa (docs, comentários, commits, PR, mensagens do xtask) em **português**.
Código — nomes de crate, tipo, função, variável e teste — em **inglês**. A saída
serial do kernel é em inglês: é saída do sistema, lida por ferramenta. Branch é
identificador: em inglês.
