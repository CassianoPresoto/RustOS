# SDD 06 — Testes

> **O que este capítulo responde:** quais andares de teste existem, onde cada um
> roda, o comando, e o que ainda não existe.

## Os andares

| Andar | O que prova | Onde roda | Comando |
| --- | --- | --- | --- |
| 1. Unidade da política | Algoritmo de `core/` certo, inclusive bordas (frame parcial, bitmap no limite, fila cheia) | host | `make test` |
| 2. Adaptador com porta falsa | Driver e caminhada de tabela fazem a sequência certa de acessos, sem hardware | host | `make test` |
| 3. Fumaça na QEMU | O kernel sobe de verdade e chega ao `boot ok` nas duas ISAs | QEMU | `make smoke ARCH=all` |
| 3b. Tela na QEMU | O que o kernel desenha: fonte, terminal, shell e barra, depois de digitar comandos | QEMU | `make screenshot ARCH=all CMDS="fetch"` |
| 4. Gates de arquitetura | Camadas, `unsafe`, `asm!`, comentários, docs | host | `make check` |

## Andar 1 — política

Todo crate de `core/` e `lib/` é `#![cfg_attr(not(test), no_std)]`: no teste, tem
`std`; no kernel, não. Exemplos: `core/mm/src/frame.rs` (alocação, dupla liberação,
faixas recortadas, palavras parciais), `core/mm/src/buddy.rs` e `heap.rs` (400 mil e
300 mil passos aleatórios contra um modelo `BTreeMap`: nada se sobrepõe, o conteúdo
sobrevive e tudo volta ao estado de partida), `lib/sync/src/spin.rs` (4 threads × 10 000
incrementos — foi esse teste que achou o `then_some` que soltava o lock alheio).

Medição de `core/mm` no host, fora do `make test` (testes `#[ignore]`):

```sh
cargo test --release -p rustos-mm bench -- --ignored --nocapture --test-threads=1
```

## Andar 2 — adaptador com porta falsa

| Teste | Porta falsa |
| --- | --- |
| `drivers/uart16550`, `drivers/pl011` | `FakeBus: RegisterBus` que grava cada escrita e simula FIFO cheia |
| `core/mm/src/tables.rs` | `Toy: PageFormat` (mesma forma dos formatos reais), `FakeTables: TableMemory` em `HashMap`, `Cpu: MmuControl` que conta as publicações e `Frames: FrameSource` em `Vec`; mapeia página de 4 KiB e de 2 MiB, recusa W+X e percorre as folhas |
| `arch/*/src/pte.rs`, `descriptor.rs` | A codificação: ida e volta de cada `PageFlags`, e o `PageFormat` que distingue tabela, folha grande e entrada vazia |
| `core/mm/src/wx.rs` | Lista de `Mapping` montada à mão: W+X, seção com permissão trocada, alias da imagem no mapa direto, metade baixa |
| `boot/limine` | Layout das structs contra `limine.h` (`size_of`, `offset_of`), tradução dos tipos de região e do formato do framebuffer |
| `core/mm/src/heap.rs` | `FakeMemory: FrameMemory`, uma faixa de palavras que derruba o teste em acesso fora dela, desalinhado ou abaixo da base |
| `core/shell` | `Host` falso e um terminal VT mínimo (`testing.rs`) que confere prompt, linha e cursor depois de cada tecla, nas duas regras de última coluna |
| `core/term` | Fuzz determinístico (xorshift, 300 mil bytes por tamanho de grade) que confere cursor, região e anel; o mesmo fluxo em bloco e byte a byte dá a mesma grade |

O que tem `asm!` só compila com `target_os = "none"`; a parte pura do mesmo crate
(`descriptor.rs`, `pte.rs`) roda no host. A caminhada de tabela é uma só, em
`core/mm`; o `mmu.rs` de cada ISA tem só o que precisa de `asm!` e de ponteiro.

## Andar 3 — fumaça

`cargo xtask smoke` monta a ESP, sobe a QEMU sem tela e lê a serial. Passa ao ver
`[rustos] boot ok`; falha com pânico (`[rustos] panic`), com a QEMU saindo antes,
ou em 120 s. O marcador é a constante `BOOT_OK_MARKER` do kernel e `BOOT_OK` do
xtask: os dois mudam juntos. O kernel só imprime o marcador se os autotestes de
frame, de heap (`heap self-test`), de W^X (`w^x: ok`) e de interrupção passarem. O de
W^X percorre as tabelas novas ([SDD 13](13_memoria_virtual.md)); `fault text` no shell
confere que a CPU recusa a escrita em `.text`.

## Andar 3b — tela

`cargo xtask screenshot` sobe a QEMU sem janela, espera o `boot ok`, digita cada
comando na serial, pede `screendump` pelo monitor (socket Unix) e converte o PPM
em PNG com o codificador do próprio xtask (`xtask/src/png.rs`). No CI, a tela de
cada ISA fica como artefato do run. Ninguém compara pixel ainda: a imagem é para
o review ver o que o kernel desenhou.

## O que ainda não existe

- **Teste dentro do kernel** (`#[ktest]` no estilo do Asterinas, com saída por
  `isa-debug-exit` no x86 e PSCI `SYSTEM_OFF` no aarch64): entra quando houver
  código de kernel que só se testa rodando — interrupções, fase 1.
- **Cobertura**: sem ferramenta, porque `cargo-llvm-cov` é crate externo
  (decisão do caso 2 do `AGENTS.md`).
- **Hardware real**: nenhum boot fora da QEMU ainda.
