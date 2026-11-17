# Guidelines 04 — `unsafe` e assembly

> **O que este tema cobre:** onde `unsafe` e `asm!` podem morar e como cada uso se
> justifica.

## Regras

- **`#![forbid(unsafe_code)]` em todo crate de `core/` e de `drivers/`.** Driver
  recebe o hardware pronto pela porta `RegisterBus`.
- **`asm!` e `global_asm!` só em `arch/`**, em módulo com
  `#[cfg(all(target_arch = "...", target_os = "none"))]`.
- **A parte pura de um adaptador fica fora do módulo com `asm!`** (como
  `descriptor.rs` e `walk.rs` no `arch/aarch64`), para rodar no teste do host.
- **Todo bloco `unsafe` tem `// SAFETY:` logo acima**, dizendo qual invariante
  torna o acesso válido. Lint `undocumented_unsafe_blocks` em `deny`.
- **Uma operação `unsafe` por bloco** (`multiple_unsafe_ops_per_block` em `deny`).
- **Toda `unsafe fn` tem `/// # Safety`** com o contrato de quem chama
  (`missing_safety_doc` em `deny`).
- **O invariante mora no construtor:** o adaptador tem `unsafe fn new(...)` que
  estabelece o contrato, e os métodos da porta são seguros. Método de porta nunca é
  `unsafe` (o `hal` é `forbid(unsafe_code)`).
- **Memória escrita fora da visão do compilador** (resposta do bootloader,
  registrador) é lida com `read_volatile` e mora em `UnsafeCell`.
- **Endereço vira ponteiro por `ptr::with_exposed_provenance_mut`**, nunca por
  `as *mut T`.
- **`asm!` sem `nomem` quando precisa ser barreira** para o compilador (`cli`,
  `msr daifset`, E/S de dispositivo).

## Como se verifica

`make layers-check` confere o `forbid` em `core/` e `drivers/` e procura `asm!` fora
de `arch/`. `make clippy` cobre `SAFETY`, um op por bloco e `# Safety`.

## O que NÃO fazer

- **`Option::then_some` com valor que tem `Drop`.** O argumento é avaliado mesmo
  quando a condição é falsa: no `SpinLock::try_lock` isso criava e descartava um
  guard na falha, e o `Drop` soltava o lock de outra thread. Use `if`/`else` ou
  `then(|| ...)`.
