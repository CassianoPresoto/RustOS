# RustOS

Sistema operacional escrito do zero em Rust, com arquitetura hexagonal e um
objetivo medível: **gastar o mínimo de memória e CPU**, rodar de hardware pequeno
a máquinas com muitos núcleos e ser comparado, com números, contra Linux e Windows.

Não é distro Linux e não usa kernel de terceiros. O kernel não tem nenhum crate
externo: só `core`, `alloc` e o que está neste repositório.

## Estado

Fase 2 — memória virtual e heap. O kernel sobe pelo Limine em **x86_64**
e **aarch64** (QEMU) e monta o alocador de frames. Desenha na tela com uma **fonte
própria** e um **emulador de terminal VT/xterm** escritos aqui, e abre um **shell
embutido** (`help`, `fetch`, `mem`, `colors`, `uptime`, `keymap`, `ps`, `bench`, histórico, Tab,
edição de linha). Exceções, controlador de interrupção (APIC ou GICv3), timer e
relógio são próprios. O shell dorme até chegar tecla, pela serial ou pelo teclado
PS/2 com layout US ou ABNT2. As proteções de hardware ficam ligadas: WP, NX,
SMEP, SMAP e UMIP no x86_64, e PAN no aarch64. Nada da interface vem de
biblioteca.

A fase 2 trouxe tabelas de página próprias com W^X, heap do kernel (buddy de
frames e slabs), gerador aleatório criptográfico (ChaCha20), KASLR e a memória
do bootloader devolvida ao alocador. Ainda não há escalonador nem espaço de
usuário.

```sh
make run ARCH=x86_64           # janela com a tela; digite nela ou no terminal
make screenshot ARCH=all CMDS="fetch;mem"   # grava target/screenshots/<isa>.png
```

O kernel carregado ocupa ~180 KiB (x86_64) e ~150 KiB (aarch64) em `release`, e
~119 KiB e ~101 KiB em `release-small`
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
                                                    core/crypto, core/gfx,
                                                    core/term, core/shell
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
