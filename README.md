# RustOS

Sistema operacional escrito do zero em Rust, com arquitetura hexagonal e um
objetivo medível: **gastar o mínimo de memória e CPU**, rodar de hardware pequeno
a máquinas com muitos núcleos e ser comparado, com números, contra Linux e Windows.

Não é distro Linux e não usa kernel de terceiros. O kernel não tem nenhum crate
externo: só `core`, `alloc` e o que está neste repositório.

## Estado

Fase 0.5 — tela e terminal. O kernel sobe pelo Limine em **x86_64** e **aarch64**
(QEMU), monta o alocador de frames, desenha na tela com uma **fonte própria** e um
**emulador de terminal VT/xterm** escritos aqui, e abre um **shell embutido**
(`help`, `fetch`, `mem`, `colors`, `uname`, histórico, Tab, edição de linha). Nada
da interface vem de biblioteca: fonte, terminal, desenho e shell são deste
repositório.

```sh
make run ARCH=aarch64          # janela com a tela; o shell lê o teclado do terminal
make screenshot ARCH=all CMDS="fetch;mem"   # grava target/screenshots/<isa>.png
```

O kernel carregado ocupa ~98 KiB em `release` e ~60 KiB em `release-small`
(medição no [SDD 07](docs/sdd/07_desempenho_e_recursos.md)). O que vem depois está
no [roadmap](docs/sdd/08_roadmap.md).

## Arquitetura

```text
                 ┌──────────────────────────── kernel/ ───────────────────────────┐
                 │  raiz de composição: escolhe qual adaptador atende cada porta  │
                 └───────┬───────────────────┬───────────────────────┬────────────┘
                         │                   │                       │
   adaptadores   arch/x86_64, arch/aarch64   boot/limine   drivers/uart16550, drivers/pl011
   (unsafe, asm)         │                   │                       │
                         └─────────┬─────────┴───────────┬───────────┘
                                   ▼                     ▼
   portas                    core/hal  ◄──────────  core/mm, core/sched,    centro do hexágono
                                                    core/gfx, core/term,
                                                    core/shell
                         (traits e tipos)          (política pura, sem unsafe, testada no host)
```

Detalhes em [`docs/sdd/02_arquitetura.md`](docs/sdd/02_arquitetura.md) e a árvore
comentada em [`docs/sdd/03_mapa_de_codigo.md`](docs/sdd/03_mapa_de_codigo.md).

## Rodando

Pré-requisitos: [rustup](https://rustup.rs) (o `rust-toolchain.toml` instala a
versão e os alvos certos), QEMU com firmware EDK2, `curl`, `tar` e `make`.

```sh
brew install qemu          # macOS; no Debian/Ubuntu: qemu-system-x86 ovmf qemu-system-arm qemu-efi-aarch64
make smoke ARCH=all        # compila, baixa o Limine, sobe nas duas ISAs e confere o boot
make run ARCH=aarch64      # serial no terminal; Ctrl-A X sai
```

Tudo o que o `make` faz passa pelo `cargo xtask` — ferramenta de host deste
repositório, também sem crate externo. Comandos em
[`docs/sdd/05_build_e_execucao.md`](docs/sdd/05_build_e_execucao.md).

## Documentação

| Onde | Responde |
| --- | --- |
| [`docs/sdd/`](docs/sdd/README.md) | O que é o projeto e como se desenvolve nele |
| [`docs/guidelines/`](docs/guidelines/README.md) | Como escrever: estilo, camadas, `unsafe`, recursos, testes |
| [`docs/tech-debt/`](docs/tech-debt/README.md) | O que se sabe que falta, com gatilho |

Assistentes de código: [`AGENTS.md`](AGENTS.md) é a fonte única de contexto.
