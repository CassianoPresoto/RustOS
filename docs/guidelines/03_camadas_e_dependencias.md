# Guidelines 03 — Camadas e dependências

> **O que este tema cobre:** quem pode depender de quem no hexágono, e o que entra
> de fora. A árvore de hoje está no [SDD 03](../sdd/03_mapa_de_codigo.md).

## Regras

1. **`core/hal` (portas) não depende de nada.**
2. **Os outros crates de `core/` (centro) dependem só de `core/hal`.** Um subsistema
   não depende de outro: se `sched` precisar de memória, pede por uma porta que o
   `kernel/` liga ao `mm`.
3. **`lib/` depende só de `lib/`.**
4. **Adaptador (`arch/`, `boot/`, `drivers/`) depende só de `core/hal` e `lib/`.**
   Nunca de outro adaptador e nunca de `core/mm` ou `core/sched`.
5. **Só o `kernel/` depende de adaptador** (`arch/<isa>`, `boot/`, `drivers/`), e nada
   depende do `kernel/`. Entre os adaptadores, o `user/` depende só dos `arch/<isa>-user`
   (regra 11), e o `kernel/` nunca depende deles (regra 12).
6. **`xtask/` não depende de crate do workspace.**
7. **Nenhum crate externo**, em nenhuma camada, nem como `dev-dependency`. A lista
   de aprovados é `ALLOWED_EXTERNAL` em `xtask/src/checks/layers.rs`, e está vazia.
8. **Crate novo mora numa das pastas acima** e é membro do `[workspace]`; pasta nova
   de primeiro nível é decisão.
9. **Toda dependência interna passa por `[workspace.dependencies]`**
   (`rustos-hal.workspace = true`), nunca por `path =` no crate.
10. **Dependência só de um alvo vai em `[target.'cfg(...)'.dependencies]`**, como o
    `hal` do `arch/x86_64` e os adaptadores por ISA do `kernel/`.
11. **`user/` é o segundo anel de composição**: runtime e programas de usuário, só para
    os alvos bare-metal. Depende de `core/hal` (módulo `abi`, o contrato de syscall), de
    `core/elf`, de `core/cpio` e de `arch/<isa>-user`. **Nada depende de `user/`**, e ele
    não depende do `kernel/` nem de `arch/<isa>`. Sem `asm!`: o ponto de entrada e os
    stubs de syscall moram em `arch/<isa>-user`. `unsafe` só no `user/rt`, mínimo e com
    `// SAFETY:` (alocador global, leitura do auxv, `#[unsafe(no_mangle)]`); os programas
    ficam em `#![forbid(unsafe_code)]`. **Exceção:** os programas de `user/tests/*` podem ter
    `unsafe`, só para syscall crua em teste hostil (`rt::sys::raw`, feature `raw-syscalls`), com
    `#![deny(unsafe_code)]` no crate e `#[allow(unsafe_code)]` pontual com `// SAFETY:`. Só ali
    a feature liga, e o xtask compila cada programa numa invocação própria do cargo, para ela
    não unificar no `init`.
12. **`arch/<isa>-user` é adaptador do lado do usuário**: segue a regra 4 (só `core/hal`
    e `lib/`), é o lugar do `asm!` de `syscall`/`svc` e do ponto de entrada, e o `kernel/`
    nunca o liga.
13. **Exceção à regra 11 (decisão 7a do [SDD 18](../sdd/18_m2_doom_nativo.md#libc-mínima)):**
    `user/libc` (a ABI C para o doomgeneric) pode ter `unsafe` só nas funções
    `extern "C"` que a ABI exige, com `// SAFETY:`. O crate tem
    `#![deny(unsafe_code)]` e `#[allow(unsafe_code)]` pontual. Nada depende de
    `user/libc` além de `user/doom` e de programas C ligados por `user/`.

## Como se verifica

`make layers-check` (`cargo xtask check-layers`) lê o `Cargo.toml` de cada membro,
classifica pela pasta e falha em dependência proibida ou crate externo. O PR da fase 4a trouxe o
`check-layers` conhecendo `user/` e `arch/<isa>-user` (regras 11 e 12), **com o teste vermelho
antes do verde**: antes dele, `arch/x86_64-user` seria classificado como `Adapter` e o `Kernel`
poderia depender dele. O mesmo comando confere a regra 11: `forbid(unsafe_code)` nos programas de
`user/`, `deny(unsafe_code)` em `user/tests/*`, e o `raw-syscalls` do `rt` só nos de teste. A regra 9:

```sh
grep -rn 'path = ' --include=Cargo.toml core lib arch boot drivers kernel xtask
```

## O que NÃO fazer

- **Fazer o driver depender do `arch/` para ter E/S.** O 16550 usa porta no x86 e
  MMIO em outras placas; a dependência certa é a porta `hal::RegisterBus`, que o
  `arch/` implementa e o `kernel/` entrega ao driver.
