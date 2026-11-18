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
5. **Só o `kernel/` depende de adaptador de kernel** (`arch/<isa>`, `boot/`, `drivers/`), e
   nada depende do `kernel/`. O `user/` depende só dos adaptadores `arch/<isa>-user`
   (regra 11).
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
    não depende do `kernel/` nem de `arch/<isa>`. Não tem `asm!`: o ponto de entrada e os
    stubs de syscall moram em `arch/<isa>-user`.
12. **`arch/<isa>-user` é adaptador do lado do usuário**: segue a regra 4 (só `core/hal`
    e `lib/`), é o lugar do `asm!` de `syscall`/`svc` e do ponto de entrada, e o `kernel/`
    nunca o liga.

## Como se verifica

`make layers-check` (`cargo xtask check-layers`) lê o `Cargo.toml` de cada membro,
classifica pela pasta e falha em dependência proibida ou crate externo. As regras 11 e 12
valem a partir do PR que cria o `user/`: é nele que o `check-layers` ganha a camada `User`. A regra 9:

```sh
grep -rn 'path = ' --include=Cargo.toml core lib arch boot drivers kernel xtask
```

## O que NÃO fazer

- **Fazer o driver depender do `arch/` para ter E/S.** O 16550 usa porta no x86 e
  MMIO em outras placas; a dependência certa é a porta `hal::RegisterBus`, que o
  `arch/` implementa e o `kernel/` entrega ao driver.
