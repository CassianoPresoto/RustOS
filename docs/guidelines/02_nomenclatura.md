# Guidelines 02 — Nomenclatura

> **O que este tema cobre:** como se nomeia crate, pasta, tipo, teste e mensagem,
> e em que idioma.

## Regras

- **Crate se chama pela camada:** `rustos-<nome>` em `core/` e `lib/`;
  `rustos-arch-<isa>`, `rustos-boot-<protocolo>`, `rustos-driver-<dispositivo>`;
  `rustos-kernel`; `xtask`. A pasta é o mesmo nome sem o prefixo.
- **Código em inglês:** crate, módulo, tipo, função, variável, constante e teste.
- **Prosa em português:** comentários, docs, commits, PR e mensagens do `xtask`.
- **Saída serial do kernel em inglês, com prefixo `[rustos]`.** É saída do sistema,
  lida por ferramenta (`make smoke` procura `[rustos] boot ok`).
- **Teste diz o comportamento em frase:** `allocates_lowest_released_frame_first`,
  não `test_alloc_1`. Sem prefixo `test_`.
- **Adaptador diz o que adapta:** `PortBus`, `MmioBus`, `Aarch64PageMapper`,
  `LimineBoot`. Porta diz o papel: `Console`, `RegisterBus`, `PageMapper`.
- **Tipo de endereço nunca é `u64` solto na fronteira:** `PhysAddr` ou `VirtAddr`.
- **Branch em inglês** (é identificador): `feat/x86-interrupts`.

## Como se verifica

```sh
grep -rn 'fn test_' --include=*.rs .
grep -n '^name = ' */*/Cargo.toml kernel/Cargo.toml xtask/Cargo.toml
```
