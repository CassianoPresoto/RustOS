# SDD 06 — Testes

> **O que este capítulo responde:** quais andares de teste existem, onde cada um
> roda, o comando, e o que ainda não existe.

## Os andares

| Andar | O que prova | Onde roda | Comando |
| --- | --- | --- | --- |
| 1. Unidade da política | Algoritmo de `core/` certo, inclusive bordas (frame parcial, bitmap no limite, fila cheia) | host | `make test` |
| 2. Adaptador com porta falsa | Driver e caminhada de tabela fazem a sequência certa de acessos, sem hardware | host | `make test` |
| 3. Fumaça na QEMU | O kernel sobe de verdade e chega ao `boot ok` nas duas ISAs | QEMU | `make smoke ARCH=all` |
| 4. Gates de arquitetura | Camadas, `unsafe`, `asm!`, comentários, docs | host | `make check` |

## Andar 1 — política

Todo crate de `core/` e `lib/` é `#![cfg_attr(not(test), no_std)]`: no teste, tem
`std`; no kernel, não. Exemplos: `core/mm/src/frame.rs` (alocação, dupla liberação,
faixas recortadas, palavras parciais), `lib/sync/src/spin.rs` (4 threads × 10 000
incrementos — foi esse teste que achou o `then_some` que soltava o lock alheio).

## Andar 2 — adaptador com porta falsa

| Teste | Porta falsa |
| --- | --- |
| `drivers/uart16550`, `drivers/pl011` | `FakeBus: RegisterBus` que grava cada escrita e simula FIFO cheia |
| `arch/aarch64/src/walk.rs` | `FakeTables: TableMemory` em `HashMap` e `Frames: FrameSource` em `Vec` |
| `boot/limine` | Layout das structs contra `limine.h` (`size_of`, `offset_of`) e tradução dos tipos de região |

O que tem `asm!` só compila com `target_os = "none"`; a parte pura do mesmo crate
(`descriptor.rs`, `walk.rs`) roda no host. É por isso que a caminhada de tabela
mora separada do `mmu.rs`.

## Andar 3 — fumaça

`cargo xtask smoke` monta a ESP, sobe a QEMU sem tela e lê a serial. Passa ao ver
`[rustos] boot ok`; falha com pânico (`[rustos] panic`), com a QEMU saindo antes,
ou em 120 s. O marcador é a constante `BOOT_OK_MARKER` do kernel e `BOOT_OK` do
xtask: os dois mudam juntos. O kernel só imprime o marcador se o autoteste de
frame passar.

## O que ainda não existe

- **Teste dentro do kernel** (`#[ktest]` no estilo do Asterinas, com saída por
  `isa-debug-exit` no x86 e PSCI `SYSTEM_OFF` no aarch64): entra quando houver
  código de kernel que só se testa rodando — interrupções, fase 1.
- **Cobertura**: sem ferramenta, porque `cargo-llvm-cov` é crate externo
  (decisão do caso 2 do `AGENTS.md`).
- **Hardware real**: nenhum boot fora da QEMU ainda.
