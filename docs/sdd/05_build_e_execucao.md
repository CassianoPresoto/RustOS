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

| Perfil | Para quê | Configuração | Modo de teste (`selftest`) |
| --- | --- | --- | --- |
| `dev` | Desenvolvimento: asserts de debug ligados | `opt-level = 1` | sim |
| `release` | Medição de desempenho e CI | `opt-level = 3`, `lto = "fat"`, `codegen-units = 1` | sim |
| `release-small` | **A imagem de produção**, hardware pequeno | `release` com `opt-level = "z"` | **não** |

**A feature `selftest` e a imagem enxuta.** O modo de teste ([SDD 04](04_boot.md#modos-de-boot)) é a feature `selftest` do kernel: com ela entram os autotestes e as medidas do boot (`heap`, resíduo de chave, `timer:`, `idle:`, `task`/`smp`/`wake self-test:`, `switch:`, `scale:`, `alloc storm:`, `zero bench:`, `random bench:`, `stacks:`, `user:`, `user self-test:`), os programas de teste de `user/tests/*` no initramfs (o xtask só os compila e os põe na imagem com a feature) e o comando `bench` do shell (a feature `bench` do `core/shell`, que a `selftest` liga). Sem ela, `rustos.selftest=1` na linha de comando é recusado com `[rustos] selftest: not in this build`, o boot segue no modo normal, e `bench` responde `bench: not in this build`, como os `fault` sem `panic-tests`. O código não é compilado, então a imagem não o carrega (tamanhos em [SDD 07](07_desempenho_e_recursos.md#imagem-enxuta-sem-o-modo-de-teste)). **Quem decide é o perfil**: `selftest` fica ligada em `dev` e `release` (o `make smoke` e o CI usam a `release`) e desligada na `release-small`. `RUSTOS_SELFTEST=1|0` troca o padrão do perfil (`1` para medir a `release-small` com ela; `0` para a `release` sem ela). O `make smoke` liga a feature em qualquer perfil, porque sem ela o modo de teste não existe. **Fora do xtask a feature fica desligada**: `cargo build -p rustos-kernel` direto e o rust-analyzer compilam a imagem enxuta e não veem o código do modo de teste (nem o `bench` do shell). Para ligá-la, `--features selftest` no `cargo`, ou `rust-analyzer.cargo.features = ["rustos-kernel/selftest"]` (ou `"selftest"` com o kernel como pacote ativo) na configuração do editor.

**Por que a `release-small` e não um perfil novo.** O perfil `release-small` já é "hardware pequeno", a imagem que se entrega; é nele que cada KiB conta. Um quarto perfil (`release-lean`) repetiria a configuração do `release` para só tirar o teste, e o `release` seguiria sendo uma imagem que ninguém entrega mas que carrega o teste; assim o CI mede com o `release` e entrega o `release-small`. Se o `release-small` um dia precisar do modo de teste em produção, a saída é `RUSTOS_SELFTEST=1`, não um perfil.

## Comandos

| `make` | `cargo xtask` | O que faz |
| --- | --- | --- |
| `make build` | `build --arch A --profile P` | Compila `rustos-kernel` para o alvo da ISA |
| `make user` | `user --arch A --profile P` | Compila os programas de `user/` (o `init`, as quatro demonstrações do shell e os três de teste) para o alvo da ISA, em `target/user/<isa>/`, com a lista inteira de flags em `CARGO_ENCODED_RUSTFLAGS` (o `pie` do `.cargo/config.toml` é do kernel e não soma), o linker script `user/link/<isa>.ld`, `-C code-model=small` e `-C lto=off` no x86_64 ([SDD 17](17_processos_isolamento_e_syscalls.md#o-que-muda-no-linker-e-no-alvo)) |
| `make image` | `image` | Compila os programas de usuário (o `init` e as demonstrações `hello`, `crash`, `sysinfo` e `count`, mais `big`, `handles`, `lazy`, `child` e `procs` se o kernel tem `selftest`) e o kernel, e monta `target/esp/<isa>/`: `EFI/BOOT/<loader>`, `EFI/BOOT/limine.conf`, `boot/rustos` e `boot/initramfs.cpio`. O cpio é gerado antes em `target/initramfs/<isa>.image.cpio` (o mesmo escritor do `make initramfs`, que grava `<isa>.cpio` e não é sobrescrito pela imagem) com os programas, na ordem da lista, modo `0755` ([SDD 17](17_processos_isolamento_e_syscalls.md#origem-decisão-4)). O kernel não depende dos programas |
| `make run` | `run` | Sobe na QEMU: a tela numa janela, o shell e a serial no terminal (`-serial mon:stdio`); Ctrl-A X sai |
| `make smoke` | `smoke` | Sobe sem tela **no modo de teste** (`rustos.selftest=1`, [SDD 04](04_boot.md#modos-de-boot)) numa imagem com a feature `selftest` (ligada à força, em qualquer perfil), lê a serial até `[rustos] boot ok`, pânico ou 120 s; imprime as linhas do kernel e o tempo de parede. Exige as linhas `protections:`, `mitigations:`, `w^x: ok`, `reclaimed:` e `kaslr: on`, `boot time:`, e a de cada autoteste e medida do modo de teste (`switch:`, `scale:`, `alloc storm:`, `zero bench:`, `random bench:`, `timer:`, `idle:`…), a do `init` (`user: init loaded (`, `user: pid 1 exited (code 0,` e a saída dele, `[user] init: ok`, que só conta com o prefixo `[user] `), a sessão do shell (`ls`, `run hello`, `run crash`, `ps`) e reprova endereço do kernel no log |
| `make smoke-fast` | `smoke-fast` | O mesmo no **modo normal**, o boot que o usuário vê: exige `boot ok`, `boot time:`, as linhas comuns e que as do modo de teste não apareçam até o prompt, **digita no shell a sessão do M1** (`ls`, `run hello`, `run crash`…, [SDD 06](06_testes.md#andar-3c--modo-normal-make-smoke-fast)) e confere as respostas; imprime o tempo do kernel até o prompt. **Na imagem sem a feature `selftest` (`PROFILE=release-small`) sobe também um segundo boot com `rustos.selftest=1`**, que tem de imprimir `selftest: not in this build`, subir normal e responder `bench: not in this build` no shell: é o que mantém a imagem enxuta testada, já que o `make smoke` não a cobre |
| `make smoke-panic` | `smoke-panic` | Derruba o kernel de propósito (pânico na entrada, falha antes dos vetores, console preso, pânico aninhado) e confere a mensagem na serial, sem endereço do kernel ([SDD 06](06_testes.md)) |
| `make screenshot CMDS="fetch;mem"` | `screenshot -- fetch mem` | Sobe sem janela, digita cada comando no shell pela serial e espera o prompt voltar (até 30 s: o `bench` leva alguns segundos), imprime o que o comando respondeu (é como se lê o `mem` num boot), captura a tela pelo `screendump` do monitor da QEMU e grava `target/screenshots/<isa>.png`. Comando que começa com `keys:` vai pelo teclado da máquina (`sendkey`), em qcodes da QEMU: `keys: e c h o spc semicolon ret` |
| `make font-preview` | `font-preview` | Desenha todos os glifos da fonte própria em `target/font-preview.png` |
| `make initramfs FILES="init prog=x" [OUT=caminho]` | `initramfs [--arch A] [--out caminho] [nome=]arquivo...` | Escreve `target/initramfs/<isa>.cpio` (ou `--out`, que vale para uma ISA só) em `newc`, com o escritor em Rust puro de `xtask/src/initramfs.rs` ([SDD 17](17_processos_isolamento_e_syscalls.md#como-o-xtask-compila-e-empacota-sem-crate-externo)). Cada arquivo entra regular, modo `0755`, com o nome dado antes do `=` ou, sem ele, o último componente do caminho. Se o argumento inteiro existe como arquivo, é caminho (um `/tmp/a=b/prog` que existe fica com o nome `prog`); senão o primeiro `=` separa o nome, que pode ter `/` (`bin/sh=busybox`), do caminho, e caminho que não existe sai como erro de leitura com esse caminho. No `make`, `OUT=` vira `--out` e `FILES` separa por espaço, então não aceita caminho com espaço. Não é o que a imagem usa: o `image` monta o próprio cpio, com os programas de `user/` |
| `make size` | `size` | Lê o ELF e soma `text`, `rodata`, `data`, `bss`, do perfil como o `make build` o monta (com `selftest` ou sem) |
| `make check` | `check-layers`, `check-comments`, `check-docs` | Mais `cargo fmt --check` e `cargo clippy -D warnings` no host e nos dois alvos, com e sem `selftest` e `debug-addrs,panic-tests` (as quatro combinações), mais o `user/rt`, o `user/init`, as quatro demonstrações, o `user/tests/big` e o `arch/<isa>-user` em cada alvo |
| `make test` | — | `cargo test` dos membros padrão (todos menos o kernel), o `core/shell` sem a feature `bench` e o `user/rt` (o auxv, o `println!` e o alocador, no host) |
| `make pr` | — | `check` + `test` + `smoke`, `smoke-fast` e `smoke-panic` nas duas ISAs, o `smoke-fast` e o `smoke-panic` da `release-small` (a imagem enxuta: o `smoke-fast` leva dois boots, cerca de 4 s no x86_64 em TCG e 0,4 s no aarch64 em HVF, fora a compilação; o `smoke-panic` leva cerca de 21 s e 8 s) nas duas, mais o aarch64 em TCG com `RUSTOS_CPU=a64fx` (bloco de `DC ZVA` de 256 bytes, [DEBT-0060](../tech-debt/0060-recuo-do-zerar-sem-cobertura-no-smoke.md)) |

### Rodar o sistema e usar o shell (M1)

```sh
make run ARCH=x86_64 PROFILE=release-small     # ou ARCH=aarch64
```

`PROFILE=release-small` é a imagem de produção: sem o modo de teste e com o carregador. Os programas de `user/`
(`init`, `hello`, `crash`, `sysinfo`, `count`) vão no initramfs de toda imagem; `big`, `handles`, `lazy`, `child` e `procs`, só na que
tem `selftest` (`dev` e `release`). O shell fala pela serial (o terminal que rodou o `make`) e, no x86_64, pelo teclado
da janela da QEMU; **no aarch64 a `virt` não tem teclado emulado, só a serial** ([DEBT-0050](../tech-debt/0050-historico-sem-rolagem-no-aarch64.md)).
Ctrl-A X encerra a QEMU. A sessão que mostra o M1:

| Digite | Faz | Saída esperada (em essência) |
| --- | --- | --- |
| `ls` | Lista o initramfs | `bytes  name`, e as linhas `init`, `hello`, `crash`, `sysinfo`, `count` |
| `run hello` | Roda e espera | `[user] hello from a user process (ring 3 on x86_64)` (ou `EL0 on aarch64`) e `exit code 0 (pid 1)` |
| `run sysinfo` | Relógio, página e aleatório | `[user] sysinfo: up N ms, page 4096 B, 3 loads, random 0x…` e `exit code 0 (pid P)` |
| `run crash` | Um programa que toca a página de guarda | `[user] crash: going down the stack…` e `killed: page fault on a read of 0x… (pid P)` (ou `write to`); **o prompt volta** |
| `run nothere` | Programa que não existe | `run: nothere: no such program (try 'ls')` |
| `run count 2` | Com argumento: conta até 2 e espera | `[user] count: 1/2`, `2/2`, `done` e `exit code 0 (pid P)` |
| `run count &` | Em segundo plano, 2,5 s | `started pid P` e o prompt na hora; `[user] count: 1/5 …` até `done` aparecem depois, colados ao prompt |
| `ps` | Tarefas e processos | A coluna `pid`; a linha `user` com o pid de `count` enquanto ele dorme |
| `run init` | O autoteste do `init`, sob demanda | `[user] init: ok (…)` e `exit code 0 (pid P)` |

**Na tela** só aparece isso (o que o programa escreve e a resposta do shell). As linhas de diagnóstico do kernel (`[rustos] user: entered ring3/el0`, `pid N exited (code 0, cpu C)`, `pid N stopped by a page fault at …`, `fault contained (pid N)`) vão **só para a serial**, que é o terminal do `make run`; quem lê o terminal as vê entre as respostas.

`make smoke-fast ARCH=all PROFILE=release-small` digita essa mesma sessão e a confere.

`A` é `x86_64` (padrão), `aarch64` ou `all`; `P` é `dev` (padrão), `release` ou
`release-small` (sem o modo de teste). No `make`: `ARCH=` e `PROFILE=`.

`cargo build` e `cargo test` sem `-p` rodam no host: o `default-members` deixa o
kernel de fora, porque ele só compila para alvo bare-metal (`compile_error!` em
`kernel/src/main.rs`).

## QEMU e firmware

A ESP é um diretório que a QEMU expõe como disco FAT (`-drive file=fat:rw:<dir>`):
não há ISO nem `xorriso`.

| | x86_64 | aarch64 |
| --- | --- | --- |
| Máquina | `-machine q35 -cpu max -m 256M` | `-machine virt,gic-version=3 -cpu max -m 256M` |
| Firmware | EDK2 em `-drive if=pflash,readonly=on` | EDK2 em `-bios`, com `-boot menu=on,splash-time=0` |
| Disco | AHCI (padrão da q35) | `virtio-blk-pci` |
| Tela | VGA padrão da q35 (GOP do OVMF) | `-device ramfb` (a `virt` não tem placa de vídeo) |
| Rede | `-nic none` | `-nic none` |
| Aceleração | `kvm` em host Linux x86_64 | `hvf` em Mac Apple Silicon, `kvm` em host Linux aarch64 |

O `limine.conf` também liga `kaslr: yes`; sem ele o Limine não sorteia a base. E traz `module_path: boot():/boot/initramfs.cpio` e, logo depois, `module_string: initramfs` (a `string` é o que o kernel procura para achar o initramfs; `module_string` é o nome canônico, `module_cmdline` o alias, `CONFIG.md` da v12.10.0). O `make smoke` exige `initramfs: 2 files,` (o `init` e o `big`) e o `make smoke-fast` o reprova.
A resolução vem do `limine.conf` (`resolution:`), por ISA em
`Arch::screen_resolution`: 1280x800 no x86_64 e 1024x768 no aarch64. Pedir
1280x800 ao GOP do `ramfb` deixa a tela em 800x600; 1024x768 ele aceita (medido).

Sem rede de propósito: a placa padrão da `virt` (virtio-net) carrega a option ROM
`efi-virtio.rom` do iPXE, que o Ubuntu só instala com o pacote `ipxe-qemu`, e a
QEMU sai antes do boot sem ela. TCG é sempre a reserva (`-accel <nativo> -accel tcg`).

**Features do kernel:** `RUSTOS_FEATURES=a,b` vira `--features a,b` em todo build do xtask, mais `selftest` onde o perfil a leva (acima). `selftest`: o modo de teste e o `bench`; `debug-addrs` imprime endereço cru ([SDD 09](09_seguranca.md#endereços-do-kernel-no-log)); `panic-tests` liga a leitura de `rustos.panic_early` na linha de comando (a `rustos.selftest` é lida em todo build) e os testes `fault lock`, `fault nested`, `fault all` e `fault held` (o `smoke-panic` a põe sozinho); a imagem de produção não os tem. `fault break`, `fault page`, `fault text` e `fault cpu` **não** dependem de `panic-tests` nem de `selftest`: existem em toda imagem, inclusive na de produção. O `make clippy` roda as quatro combinações de `selftest` e `debug-addrs,panic-tests`, nos dois alvos.

**Núcleos:** a QEMU sobe com `-smp 4`; `RUSTOS_SMP=N` troca o número (o kernel guarda estado para 8, `MAX_CPUS` de `core/hal`; o resto fica parado). O `make smoke` exige `[rustos] cpus: N online` com esse N. No TCG, neste QEMU 11.1.3 em Apple Silicon, os vCPUs do x86_64 rodam em threads do hospedeiro (observado: 10 threads e cerca de 280% de CPU durante o `bench`); `RUSTOS_ACCEL=tcg-multi` força `-accel tcg,thread=multi`.

`RUSTOS_ACCEL=tcg|tcg-multi|hvf|kvm` troca a lista inteira por esse acelerador, sem reserva
(`xtask/src/arch.rs`): serve para rodar o TCG no Mac, o caminho do CI (que não
tem HVF): `RUSTOS_ACCEL=tcg make smoke ARCH=aarch64`. Valor desconhecido é ignorado, com aviso. `RUSTOS_CPU=<modelo>` troca o `-cpu max` (por exemplo `RUSTOS_ACCEL=tcg RUSTOS_CPU=a64fx make smoke ARCH=aarch64`, que tem `DC ZVA` de 256 bytes). O `gic-version=3` é fixo porque, sem ele, a
`virt` escolhe a versão pelo acelerador (TCG: v2; HVF com o vGIC da Apple: v3).

**Espera do EDK2 no aarch64 (`-boot menu=on,splash-time=0`).** O EDK2 do `ArmVirtQemu` espera 5 s no gerenciador de boot antes de iniciar a
primeira opção (`PcdPlatformBootTimeOut|5` em `ArmVirtPkg/ArmVirtQemu.dsc`; o laço é o `BdsWait` de `BdsEntry.c`). O `OvmfPkg/Library/PlatformBootManagerLibLight/PlatformBm.c` pergunta o
tempo à QEMU por `GetFrontPageTimeoutFromQemu()` (`OvmfPkg/Library/QemuBootOrderLib/QemuBootOrderLib.c`), que lê `etc/boot-menu-wait` do
fw_cfg e devolve `splash-time/1000`, arredondado para cima. Com `menu=off`, o padrão da QEMU, ele devolve o PCD (5 s). Medido (HVF): o
`firmware` do `boot timeline:` vai de 5,1 s a 0,10 s e a parede do `smoke-fast` de 5,2 s a 0,17 s (TCG: 5,7 s a 0,6 s). O boot continua UEFI e
Limine. O `-kernel` do Linux já pulava a espera. No x86_64 a flag não muda nada (OVMF: `firmware` 1,32 s com e sem ela), e ela só vai no
aarch64. O `smoke-fast` do aarch64 falha se o `firmware` passar de 4 s (`xtask/src/qemu.rs`): a espera que ele pega vale 5 s ou mais, e o CI (TCG num runner x86) dá 0,6 a 0,9 s.

**Por que não uma NVRAM com `Timeout = 0`.** Também zera a espera, mas é um armazém de variáveis do EDK2 montado à mão no xtask (pflash,
`snapshot=on`, código completado até 64 MiB): depende do layout do EDK2 do pacote e, se ele diferir, o EDK2 refaz a NVRAM e a espera volta
calada. Descartada pela flag, que é do contrato da QEMU com o firmware.

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
| `smoke (x86_64)`, `smoke (aarch64)` | `make smoke`, `make smoke-fast`, `make smoke-panic`, `make size` e `make screenshot CMDS="fetch;colors"` em `release`, mais `make smoke-fast`, `make smoke-panic` e `make size` em `release-small` (a imagem enxuta), com QEMU e EDK2 do Ubuntu; a tela fica como artefato `tela-<isa>` do run |
