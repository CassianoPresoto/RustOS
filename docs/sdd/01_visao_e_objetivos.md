# SDD 01 — Visão e objetivos

> **O que este capítulo responde:** o que o RustOS quer ser, quais metas se medem
> e o que fica de fora de propósito.

## O que é

Um sistema operacional escrito do zero em Rust. Não é distro Linux, não embute
kernel de terceiros e o kernel não usa crate externo: só `core`, `alloc` e o que
está neste repositório. A única peça de terceiros no caminho do boot é o
bootloader Limine, um binário à parte (DEBT-0001).

## Metas

| Meta | Como se verifica |
| --- | --- |
| **Pouca memória** | `make size` (bytes carregados do kernel) e o relatório de frames do boot; metadado por frame comparado com o do Linux ([SDD 07](07_desempenho_e_recursos.md)). O alvo de memória ociosa **não está fixado**: nasce de uma linha de base de Linux mínimo medida na mesma QEMU, mediana de 3 boots ([SDD 08](08_roadmap.md#memória-ociosa-o-alvo-vem-de-uma-medida)) |
| **Pouca CPU** | Microbenchmarks com o mesmo método nos três sistemas: syscall nula, troca de contexto, page fault, IPC (a partir da fase 3) |
| **Hardware pequeno e grande** | O mesmo núcleo roda em x86_64 e aarch64 hoje; estruturas por CPU e sem lock global quando existir SMP (fase 3) |
| **Arquitetura limpa** | Regras de dependência e de `unsafe` checadas por máquina (`make layers-check`) |
| **Comparável com Linux e Windows** | Rodar uma aplicação Linux sem recompilar (fase 5) e medir a mesma carga nos três |

## Não-objetivos, por enquanto

- Escrever bootloader próprio: o Limine resolve o boot nas duas ISAs (DEBT-0001).
- Rodar binário Windows: a comparação com Windows é por carga equivalente, não
  por compatibilidade de binário.
- Interface gráfica antes da fase 11: até lá, a tela é o terminal da fase 0.5. O
  jogo por software (DOOM, marco 1) roda antes, no framebuffer, sem compositor.
- Jogos 3D acelerados antes da fase 13: dependem de Mesa e da ABI Linux, e ficam
  atrás da escada de jogos do [SDD 08](08_roadmap.md#jogos-a-escada).
- Suporte a 32 bits: as duas ISAs de hoje são de 64 bits; hardware pequeno de 32
  bits (Cortex-M, RV32) é decisão futura, pelo caminho de *Decisões* do `AGENTS.md`.

## Estado atual

Fase 2 do [roadmap](08_roadmap.md): o kernel sobe nas duas ISAs pela QEMU, com
tela com fonte própria, terminal VT, shell embutido, exceções, interrupções,
timer, relógio e teclado PS/2, além de:

- **Memória:** buddy de frames, heap do kernel e tabelas de página próprias com
  W^X.
- **Segurança:** gerador aleatório criptográfico e KASLR.
- **Boot:** a memória do bootloader volta para o alocador.

Não há escalonamento ligado nem espaço de usuário.
