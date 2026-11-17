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
| Linker | `rust-lld`, com `kernel/linker/<isa>.ld`, `-z max-page-size=0x1000` e `-pie` (no aarch64, também `-z notext`, DEBT-0024) | `kernel/build.rs` |
| Relocação | `pie`: o Limine relocaliza e sorteia a base ([SDD 13](13_memoria_virtual.md)) | `.cargo/config.toml` |

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
| `make run` | `run` | Sobe na QEMU: a tela numa janela, o shell e a serial no terminal (`-serial mon:stdio`); Ctrl-A X sai |
| `make smoke` | `smoke` | Sobe sem tela, lê a serial até `[rustos] boot ok`, pânico ou 120 s; imprime as linhas do kernel e o tempo de parede. Exige também as linhas `protections:`, `w^x: ok` e `reclaimed:` |
| `make screenshot CMDS="fetch;mem"` | `screenshot -- fetch mem` | Sobe sem janela, digita cada comando no shell pela serial, captura a tela pelo `screendump` do monitor da QEMU e grava `target/screenshots/<isa>.png`. Comando que começa com `keys:` vai pelo teclado da máquina (`sendkey`), em qcodes da QEMU: `keys: e c h o spc semicolon ret` |
| `make font-preview` | `font-preview` | Desenha todos os glifos da fonte própria em `target/font-preview.png` |
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
| Máquina | `-machine q35 -cpu max -m 256M` | `-machine virt,gic-version=3 -cpu max -m 256M` |
| Firmware | EDK2 em `-drive if=pflash,readonly=on` | EDK2 em `-bios` |
| Disco | AHCI (padrão da q35) | `virtio-blk-pci` |
| Tela | VGA padrão da q35 (GOP do OVMF) | `-device ramfb` (a `virt` não tem placa de vídeo) |
| Rede | `-nic none` | `-nic none` |
| Aceleração | `kvm` em host Linux x86_64 | `hvf` em Mac Apple Silicon, `kvm` em host Linux aarch64 |

O `limine.conf` também liga `kaslr: yes`; sem ele o Limine não sorteia a base.
A resolução vem do `limine.conf` (`resolution:`), por ISA em
`Arch::screen_resolution`: 1280x800 no x86_64 e 1024x768 no aarch64. Pedir
1280x800 ao GOP do `ramfb` deixa a tela em 800x600; 1024x768 ele aceita (medido).

Sem rede de propósito: a placa padrão da `virt` (virtio-net) carrega a option ROM
`efi-virtio.rom` do iPXE, que o Ubuntu só instala com o pacote `ipxe-qemu`, e a
QEMU sai antes do boot sem ela. TCG é sempre a reserva (`-accel <nativo> -accel tcg`).

`RUSTOS_ACCEL=tcg|hvf|kvm` troca a lista inteira por esse acelerador, sem reserva
(`xtask/src/arch.rs`): serve para rodar o TCG no Mac, o caminho do CI (que não
tem HVF): `RUSTOS_ACCEL=tcg make smoke ARCH=aarch64`. Valor desconhecido é ignorado, com aviso. O `gic-version=3` é fixo porque, sem ele, a
`virt` escolhe a versão pelo acelerador (TCG: v2; HVF com o vGIC da Apple: v3).

O firmware é procurado nos
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
| `smoke (x86_64)`, `smoke (aarch64)` | `make smoke`, `make size` e `make screenshot CMDS="fetch;colors"` em `release`, com QEMU e EDK2 do Ubuntu; a tela fica como artefato `tela-<isa>` do run |
