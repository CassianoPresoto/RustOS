# Guidelines 07 — Testes

> **O que este tema cobre:** o que precisa de teste e em que andar. Os andares estão
> no [SDD 06](../sdd/06_testes.md).

## Regras

- **Todo crate de `core/` e `lib/` é `#![cfg_attr(not(test), no_std)]`** e tem
  teste de unidade no host.
- **Função pública de `core/` tem teste**, inclusive das bordas: vazio, cheio,
  desalinhado, fora da faixa, transbordo.
- **Driver é testado com `RegisterBus` falso** que grava os acessos; o teste afirma
  a sequência que o datasheet pede (ex.: PL011 desliga antes de mudar a linha).
- **Lógica de `arch/` que não precisa de `asm!` sai para módulo puro** e é testada
  com porta falsa (`TableMemory`, `FrameSource`).
- **Struct de protocolo externo tem teste de layout** (`size_of`, `offset_of`) contra
  o header de referência.
- **PR que mexe em `kernel/`, `arch/`, `boot/` ou `drivers/` passa no
  `make smoke ARCH=all`** antes do review, e diz isso no PR.
- **Concorrência se testa com threads de verdade no host**: `SpinLock` tem teste de
  N threads incrementando.
- **Código de teste não entra no binário do kernel**: só em `#[cfg(test)]`.

## Como se verifica

`make test`, `make smoke ARCH=all`, e o CI nos dois. Crate de `core/` sem teste:

```sh
for crate in core/*/; do grep -rlq '#\[test\]' "$crate" || echo "sem teste: $crate"; done
```
