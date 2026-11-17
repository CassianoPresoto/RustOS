# Guidelines 01 — Estilo

> **O que este tema cobre:** que ferramenta decide o estilo, o que falha o build e
> como abrir exceção.

## Regras

- **`rustfmt` é a fonte da verdade** da formatação, com `rustfmt.toml` da raiz.
  Não se discute formatação em review.
- **`clippy` com `pedantic` ligado, e aviso é erro** (`-D warnings`) no host e nos
  dois alvos bare-metal. Os lints moram em `[workspace.lints]` do `Cargo.toml`;
  todo crate tem `[lints] workspace = true`.
- **Exceção a lint é local e diz o porquê:** `#[allow(clippy::x, reason = "...")]`
  no item, nunca no crate inteiro. A exceção do crate inteiro só existe no `xtask`
  (`std_instead_of_core`, porque ele é ferramenta de host).
- **Lint desligado no workspace tem uma linha de motivo** ao lado, no
  `Cargo.toml`.
- **Edição 2024 e Rust estável fixado** em `rust-toolchain.toml`. Nightly e
  `#![feature]` são decisão (caso 2 do `AGENTS.md`).
- **Cadeia de `if let` em vez de `if` aninhado** (`if let A && let B`), como o
  `clippy::collapsible_if` pede na edição 2024.

## Como se verifica

`make check` roda `cargo fmt --all --check` e `cargo clippy -- -D warnings` no host
e para `x86_64-unknown-none` e `aarch64-unknown-none-softfloat`. Exceção sem motivo:

```sh
grep -rn '#\[allow(' --include=*.rs . | grep -v 'reason ='
```

## O que NÃO fazer

- **Rodar o clippy só no host.** O código com `asm!` só compila para o alvo
  bare-metal; lint dele (`trivially_copy_pass_by_ref` no `PortBus`,
  `is_multiple_of` no `MmioBus`) só aparece com `--target`.
