# SDD 05 — Build e execução

> **O que este capítulo responde:** com que toolchain o projeto compila, para quais
> alvos, e o que cada comando do `make` e do `cargo xtask` faz.

## Toolchain e alvos

| | Valor | Onde |
| --- | --- | --- |
| Rust | 1.100.0 estável, edição 2024 | `rust-toolchain.toml`, `Cargo.toml` |
| Componentes | `rustfmt`, `clippy`, `rust-src`, `llvm-tools` | `rust-toolchain.toml` |
| Alvo x86_64 | `x86_64-unknown-none` (soft-float, sem red zone, code model kernel) | `xtask/src/arch.rs` |
| Alvo aarch64 | `aarch64-unknown-none-softfloat` | `xtask/src/arch.rs` |
| Linker | `rust-lld`, com `kernel/linker/<isa>.ld` e `-z max-page-size=0x1000` | `kernel/build.rs` |
| Relocação | `static` (kernel em endereço fixo da metade alta) | `.cargo/config.toml` |

Os dois alvos são Tier 2 com `core` pré-compilado: não há `build-std` nem nightly.

## Perfis

| Perfil | Para quê | Configuração |
| --- | --- | --- |
| `dev` | Desenvolvimento: asserts de debug ligados | `opt-level = 1` |
| `release` | Medição de desempenho | `opt-level = 3`, `lto = "fat"`, `codegen-units = 1` |
| `release-small` | Hardware pequeno | `release` com `opt-level = "z"` |

## Comandos

| `make` | `cargo xtask` | O que faz |
| --- | --- | --- |
| `make build` | `build --arch A --profile P` | Compila `rustos-kernel` para o alvo da ISA |
| `make image` | `image` | Monta `target/esp/<isa>/`: `EFI/BOOT/<loader>`, `EFI/BOOT/limine.conf`, `boot/rustos` |
| `make run` | `run` | Sobe na QEMU com `-serial mon:stdio`; Ctrl-A X sai |
| `make smoke` | `smoke` | Sobe sem tela, lê a serial até `[rustos] boot ok`, pânico ou 120 s; imprime as linhas do kernel e o tempo de parede |
| `make size` | `size` | Lê o ELF e soma `text`, `rodata`, `data`, `bss` |
| `make check` | `check-layers`, `check-comments`, `check-docs` | Mais `cargo fmt --check` e `cargo clippy -D warnings` no host e nos dois alvos |
| `make test` | — | `cargo test` dos membros padrão (todos menos o kernel) |
| `make pr` | — | `check` + `test` + `smoke` nas duas ISAs |

`A` é `x86_64` (padrão), `aarch64` ou `all`; `P` é `dev` (padrão), `release` ou
`release-small`. No `make`: `ARCH=` e `PROFILE=`.

`cargo build` e `cargo test` sem `-p` rodam no host: o `default-members` deixa o
kernel de fora, porque ele só compila para alvo bare-metal (`compile_error!` em
`kernel/src/main.rs`).

## QEMU e firmware

A ESP é um diretório que a QEMU expõe como disco FAT (`-drive file=fat:rw:<dir>`):
não há ISO nem `xorriso`.

| | x86_64 | aarch64 |
| --- | --- | --- |
| Máquina | `-machine q35 -cpu max -m 256M` | `-machine virt -cpu max -m 256M` |
| Firmware | EDK2 em `-drive if=pflash,readonly=on` | EDK2 em `-bios` |
| Disco | AHCI (padrão da q35) | `virtio-blk-pci` sem option ROM (`romfile=`): o EDK2 já tem driver virtio, e o Ubuntu só traz a ROM do iPXE no pacote `ipxe-qemu` |
| Aceleração | `kvm` em host Linux x86_64 | `hvf` em Mac Apple Silicon, `kvm` em host Linux aarch64 |

TCG é sempre a reserva (`-accel <nativo> -accel tcg`). O firmware é procurado nos
caminhos do Homebrew, Debian/Ubuntu e Fedora/Arch (`xtask/src/arch.rs`), ou nas
variáveis `RUSTOS_FIRMWARE_X86_64` e `RUSTOS_FIRMWARE_AARCH64`.

## Limine

`cargo xtask image` baixa `limine-binary.tar.gz` da release fixada no GitHub uma
vez, para `target/limine/<versão>/`, confere o SHA-256 (`sha256sum` ou `shasum`) e
extrai. Hash diferente apaga o arquivo e falha. Trocar de versão é mudar `VERSION`
e `SHA256` em `xtask/src/limine.rs` no mesmo PR.

## CI

`.github/workflows/ci.yml`, em todo push na `main` e todo PR:

| Job | Roda |
| --- | --- |
| `check` | `make check` e `make test` |
| `smoke (x86_64)`, `smoke (aarch64)` | `make smoke` e `make size` em `release`, com QEMU e EDK2 do Ubuntu |
