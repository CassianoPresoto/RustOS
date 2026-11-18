# SDD 09 — Segurança

> **O que este capítulo responde:** de quem o RustOS se defende, com que
> mecanismos, em que fase cada um entra e quanto custa, e quais decisões de
> segurança ainda estão abertas.

## Princípios

| Princípio | Como aparece no código |
| --- | --- |
| **Memória segura por construção** | Rust em tudo; `unsafe` só em `arch/`, `boot/`, `lib/` e `kernel/`, com `// SAFETY:` ([guidelines 04](../guidelines/04_unsafe_e_asm.md)). `core/` e `drivers/` são `forbid(unsafe_code)` |
| **Menor privilégio, sem autoridade ambiente** | Um processo só acessa o que recebeu como handle, com direitos que só diminuem (fase 4) |
| **Dado de fora é validado na fronteira** | Resposta do bootloader, registrador de dispositivo e argumento de syscall viram tipo do núcleo só depois de conferidos. Exemplo de hoje: `framebuffer_info` em `boot/limine` recusa formato que o desenho não entende |
| **Barato e difícil de pôr depois entra cedo** | W^X, NX, SMEP/SMAP, PAN e páginas de guarda entram nas fases 1 e 2, antes de existir usuário; as mitigações de CPU entram com o primeiro código de usuário (fase 4), porque mexem na entrada de syscall |
| **Caro entra medido** | Mitigação com custo (KPTI, retpoline, MTE) entra com o número no [SDD 07](07_desempenho_e_recursos.md) e liga só onde a CPU precisa |
| **Padrão seguro** | Rede nega o que entra por padrão; processo nasce sem handle nenhum; segredo nunca vai para log nem serial |

## Modelo de ameaça

| Atacante | O que pode fazer | Defesa | Fase |
| --- | --- | --- | --- |
| Programa de usuário malicioso | Ler ou escrever memória de outro processo ou do kernel, escalar privilégio, esgotar recurso | Espaço de endereçamento por processo, SMEP/SMAP/PAN, handles com direitos, grupos de recursos | 4, 9 |
| Binário Linux com bug explorável | Estouro de buffer, ROP | ASLR, NX, CET/PAC/BTI, filtro de syscalls | 4, 12 |
| Entrada de rede | Pacote malformado, inundação | Pilha TCP/IP em Rust seguro, validação na fronteira, filtro que nega por padrão | 10 |
| Disco ou imagem malformada | Sistema de arquivos corrompido de propósito | Parser em `core/` sem `unsafe`, com fuzz | 6 |
| Dispositivo malicioso (DMA) | Escrever na memória do kernel por DMA | IOMMU (VT-d, SMMU) | 6, antes do primeiro driver com DMA em metal |
| Canal lateral de CPU | Meltdown, Spectre, MDS, LazyFP | Mitigações por CPU, relatadas no boot; estado FPU salvo eager desde a fase 3 | 3, 4 |
| Acesso físico, firmware adulterado | Trocar kernel ou bootloader; ler o disco de uma máquina roubada | Boot verificado (Secure Boot, TPM); criptografia de disco no instalador | 8, 12, decisões abertas |

Fora de escopo por enquanto: ataque físico a memória (cold boot), canal lateral de
energia e firmware malicioso abaixo do UEFI.

## Mecanismos, por fase

"A medir" quer dizer que não há número publicado que sirva, e o custo entra no
SDD 07 quando o mecanismo entrar. "Não lido nesta rodada" quer dizer que a
afirmação veio de fonte secundária e precisa da leitura do manual antes de virar
código.

| Mecanismo | Protege contra | Fase | Custo | Fonte |
| --- | --- | --- | --- | --- |
| WP (x86_64) | Kernel escrever em página só de leitura | 1 | Desprezível (não lido nesta rodada) | Intel SDM vol. 3 (não lido nesta rodada) |
| SMEP, SMAP (x86_64); PAN (aarch64) | Kernel executar (SMEP) ou ler e escrever (SMAP, PAN) página de usuário por engano | 1 | Desprezível (não lido nesta rodada) | Intel SDM vol. 3, Arm ARM (não lidos nesta rodada) |
| UMIP (x86_64) | `sgdt`/`sidt` em modo usuário vazando endereço do kernel | 1 | Desprezível (não lido nesta rodada) | Intel SDM vol. 3 (não lido nesta rodada) |
| Pilhas de exceção separadas | Exceção com a pilha corrompida derrubar a máquina sem relatório | 1 | 1 página por pilha, mais 1 de guarda | x86_64: IST só para #DF, NMI e #MC, em [Linux kernel stacks](https://docs.kernel.org/arch/x86/kernel-stacks.html); a IRQ cai na pilha da tarefa. aarch64: o kernel roda em EL1h, com `SP_EL1` por tarefa, e `SP_EL0` é a pilha de falha: as exceções síncronas e o SError trocam para ela, e a IRQ fica na pilha da tarefa (desenho do RustOS, como o `__bad_stack` do Linux; Arm ARM não lido nesta rodada). A página de guarda abaixo de cada pilha é desenho do RustOS, a criar na fase 1 |
| PXN/UXN nas tabelas próprias (aarch64) | Kernel executar página de usuário; usuário executar página do kernel | 2 (ligado: toda folha com UXN, dado com PXN) | Desprezível (não lido nesta rodada) | Arm ARM (não lido nesta rodada) |
| W^X no kernel, NX em dado | Injetar e executar código no kernel | 2 | Ligado. O mapa direto usa 2 MiB e nunca é executável; a imagem tem uma página de 4 KiB por permissão. O número de TLB espera o metal ([SDD 13](13_memoria_virtual.md)) | [kernel self-protection](https://docs.kernel.org/security/self-protection.html) |
| Aleatório do kernel: semente de RDSEED (x86_64) ou RNDR (aarch64), gerador ChaCha20 com *fast key erasure* (`core/crypto`) | Previsibilidade de endereço, chave e canário | 2 | Estado de 40 bytes; `next_u64` 67 ns e bloco de 4 KiB a 960 MB/s no host, escalar (Apple M5); no alvo (aarch64, HVF, `release`) 636 a 694 MB/s e 37 us por reposição ([SDD 07](07_desempenho_e_recursos.md)) | [RFC 8439](https://www.rfc-editor.org/rfc/rfc8439.html) (ChaCha20), [RFC 7693](https://www.rfc-editor.org/rfc/rfc7693.html) (BLAKE2b), [Bernstein, 2017](https://blog.cr.yp.to/20170723-random.html) (fast key erasure), [`random.c` do Linux](https://github.com/torvalds/linux/blob/master/drivers/char/random.c) (referência); RDSEED e RNDR conforme o Linux e a QEMU (seção abaixo; o Intel SDM e o Arm ARM não foram lidos). CPU sem os dois: jitter do contador, fraco ([DEBT-0019](../tech-debt/0019-aleatorio-sem-fonte-de-hardware-fraco.md)) |
| KASLR | Endereço fixo do kernel facilitando ROP | 2 | Ligado: +3 a 6 KiB (1,8% a 6,6%) de imagem, boot sem diferença mensurável. Base e mapa direto sorteados pelo Limine; o log não traz o endereço ([seção abaixo](#endereços-do-kernel-no-log)) | [x86_64 mm](https://docs.kernel.org/arch/x86/x86_64/mm.html); PROTOCOL.md do Limine, *Memory Layout at Entry* |
| Zerar memória ao liberar | Vazar dado de um processo para outro | 2 | Ligado por padrão. Host, cache quente: cerca de 7 ns por KiB. No alvo (aarch64, HVF, `release`): +2 ns até 64 B; com `DC ZVA` a partir de 512 B, a página custa cerca de 22 ns (eram 130 com o `memset`), 5,5 ns por KiB (SDD 07). Política `KernelHeap::set_zero_on_free` | [kernel-parameters](https://www.kernel.org/doc/html/latest/admin-guide/kernel-parameters.html) (`init_on_free`) |
| Estado FPU/SIMD salvo eager na troca de contexto (x86_64) | LazyFP (CVE-2018-3665): leitura especulativa dos registradores da tarefa anterior antes do #NM, o que vazou até chaves de round do AES-NI. Verificado do Sandy Bridge ao Skylake | 3 | Sem penalidade conhecida segundo o relato do achado (Cyberus). Medir aqui: `XSAVEOPT` por troca (SDD 07). Segundo o Cyberus, versões do Linux a partir da 4.9 não são afetadas | [Cyberus](https://www.cyberus-technology.de/post/intel-lazyfp-vulnerability-exploiting-lazy-fpu-state-switching), [Red Hat CVE-2018-3665](https://access.redhat.com/security/cve/CVE-2018-3665); [commit ca6938a1 (outubro de 2016, janela da 4.9, "x86/fpu: Hard-disable lazy FPU mode"; a mensagem diz que o eager já era o padrão desde a 4.6)](https://github.com/torvalds/linux/commit/ca6938a1cd8a1c5e861a99b67f84ac166fc2b9e7); o aviso Intel SA-00145, que recomenda eager, foi lido só por resumo de busca (não verificado) |
| Estado FPSIMD salvo eager na troca de contexto (aarch64) | Nada que seja defesa: o LazyFP é da Intel. O motivo é simplicidade e custo (512 B sempre salvos, sem trap por tarefa); a pesquisa recomenda assim e não mediu a alternativa | 3 | A medir | [Linux `fpsimd.c`](https://raw.githubusercontent.com/torvalds/linux/master/arch/arm64/kernel/fpsimd.c): o Linux também não trapeia o FPSIMD: marca `TIF_FOREIGN_FPSTATE` e recarrega ao voltar ao usuário se outra tarefa ocupou a CPU |
| `CR4.TSD = 0` (`rdtsc` liberado em modo usuário), com opção por processo para bloquear | Nada: é abrir mão de uma defesa. `rdtsc` em usuário é fonte de temporizador fino para canal lateral; o Linux também deixa liberado e oferece `PR_SET_TSC` por thread | 4 | Jogos e benchmarks leem o contador; bloquear custa um #GP por leitura e a troca do bit por processo. A medir. Opção por processo: decisão da fase 4 (o equivalente do `PR_SET_TSC` é interface, caso 6 do `AGENTS.md`) | [Intel `rdtsc` (SDM espelhado)](https://www.felixcloutier.com/x86/rdtsc) (secundária); `cpu/common.c` e `process.c` do Linux. Em aarch64, o acesso de EL0 ao contador (`CNTKCTL_EL1`): não verificado |
| Espaço de endereçamento por processo | Um processo ler outro | 4 | A medir: troca de tabela a cada troca de contexto | Desenho do RustOS (SDD 08, fase 4) |
| ASLR de usuário | Endereço fixo de pilha, heap e bibliotecas facilitando ROP | 4 | A medir | [sysctl kernel](https://docs.kernel.org/admin-guide/sysctl/kernel.html) (`randomize_va_space`) |
| Handles com direitos | Autoridade ambiente | 4 | A medir | [Fuchsia handles](https://fuchsia.dev/fuchsia-src/concepts/kernel/handles) |
| Clamp de índice, `array_index_nospec` | Spectre v1: leitura especulativa fora da tabela | 4 (**ligado** no número da syscall) | `asm!` em `arch/<isa>` com a sequência do Linux (`cmp; sbb` e `cmp; sbc; csdb`); no aarch64 a `CSDB` custa cerca de 10 ns por syscall no HVF, o resto fica no ruído ([SDD 07](07_desempenho_e_recursos.md)) | [`nospec.h` e `barrier.h` do Linux](https://github.com/torvalds/linux/blob/v6.12/include/linux/nospec.h) (v6.12) |
| Detecção de vulnerabilidade e a linha `mitigations:` | Saber o que a CPU precisa e dizer o que o kernel liga | 4 (**ligada**) | Uma linha no boot, cerca de 1 ms no x86_64 em TCG (o CPUID por núcleo é emulado) | `common.c`, `proton-pack.c` e `cpufeature.c` do Linux na v6.12 ([SDD 17](17_processos_isolamento_e_syscalls.md#mitigações-de-cpu)) |
| KPTI (só CPU vulnerável a Meltdown) | Meltdown | 4 (detectado e relatado; **não implementado**, [DEBT-0100](../tech-debt/0100-kpti-nao-implementado.md)) | ~100 ciclos por troca de CR3; sem PCID, esvazia a TLB | [Linux PTI](https://docs.kernel.org/arch/x86/pti.html) |
| Retpoline/eIBRS, IBPB, STIBP | Spectre v2 | 4 (detectado; **não implementado**, [DEBT-0101](../tech-debt/0101-spectre-v2-nao-implementado.md)) | Sem número na fonte | [Linux spectre](https://docs.kernel.org/admin-guide/hw-vuln/spectre.html) |
| Limpeza de buffers da CPU na troca de privilégio | MDS | 4 (detectado; **não implementado**, [DEBT-0103](../tech-debt/0103-mds-nao-implementado.md)) | Desligar SMT tem "impacto significativo", sem número na fonte | [Linux MDS](https://docs.kernel.org/admin-guide/hw-vuln/mds.html) |
| SSBD/SSBS, BHI/Spectre-BHB | Spectre v4 e injeção de histórico de desvio | 4 (detectados; **não implementados**, [DEBT-0102](../tech-debt/0102-ssb-nao-implementado.md) e [DEBT-0104](../tech-debt/0104-bhb-nao-implementado.md)) | A medir | `proton-pack.c` e `common.c` do Linux na v6.12 |
| Credenciais Linux (uid/gid, capabilities(7)) | Programa Linux esperar o modelo Unix | 5 | — | [capabilities(7)](https://man7.org/linux/man-pages/man7/capabilities.7.html) |
| Permissões de arquivo em disco (dono, grupo, modo) | Usuário ler arquivo de outro | 6 | A medir | POSIX (não lido nesta rodada) |
| Argon2id para senha | Quebra de senha vazada por força bruta | 7 | ~64 MiB e t=3 por login no perfil de memória limitada; em máquina com menos RAM, menos memória e mais passadas (seção 4 do RFC) | [RFC 9106](https://www.rfc-editor.org/rfc/rfc9106.html) |
| IOMMU (VT-d, SMMU) | Dispositivo fazer DMA fora da memória que o driver entregou (inferência: a fonte descreve a tradução de DMA) | 6 | A medir | [Linux x86 IOMMU](https://docs.kernel.org/arch/x86/iommu.html) |
| Criptografia de disco (opcional no instalador) | Ler o disco fora da máquina | 8 | A medir: custo por bloco de E/S | — (decisão aberta, SDD 08) |
| Grupos de recursos (CPU, memória, processos, E/S) | Um processo esgotar a máquina | 9 | A medir | [cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html) |
| CET shadow stack e IBT | ROP e JOP no x86_64 | 12 | — | [Linux shstk](https://docs.kernel.org/arch/x86/shstk.html) |
| PAC | ROP no ARM | 12 | — | [Linux PAC](https://www.kernel.org/doc/html/latest/arch/arm64/pointer-authentication.html) |
| BTI | JOP no ARM | 12 | — | [arm64 hwcaps](https://docs.kernel.org/arch/arm64/elf_hwcaps.html) (`HWCAP2_BTI`; a página só anuncia o recurso) |
| MTE | Uso depois de liberar, estouro de heap | 12 | Sem número na fonte | [Linux MTE](https://www.kernel.org/doc/html/latest/arch/arm64/memory-tagging-extension.html) |
| CFI | Desvio de chamada indireta | 12 | A medir | [LWN](https://lwn.net/Articles/1056601/) (não lido nesta rodada) |
| Filtro de syscalls por processo | Programa comprometido chamar o que não precisa | 12 | A medir | [seccomp](https://docs.kernel.org/userspace-api/seccomp_filter.html), [pledge](https://man.openbsd.org/pledge) |
| Controle de acesso obrigatório (MAC) | Programa com direito legítimo fazer o que a política proíbe | 12 | A medir | [Linux LSM](https://www.kernel.org/doc/html/latest/security/lsm.html) |
| Boot verificado e medido | Kernel ou bootloader trocados | 12 | Tempo de boot a medir | [TPM 2.0](https://trustedcomputinggroup.org/resource/tpm-library-specification/) |

Levantamento de novembro de 2026.

## Mitigações de CPU no boot

A linha `[rustos] mitigations:` fecha a regra do [guidelines 09](../guidelines/09_seguranca.md)
("o relatório de boot diz o que ficou ligado") para as vulnerabilidades de execução especulativa. Ela
sai nos dois modos, logo depois de `protections:`, e o `make smoke` e o `make smoke-fast` a exigem
nas duas ISAs. **O que ela promete:** `on` quer dizer que o kernel liga a mitigação **agora**
(hoje só o `v1`, o clamp do Spectre v1); `on (hardware)` que a CPU é afetada e o próprio hardware a mitiga, sem ação do kernel (o BHB com `ECBHB` no aarch64; não é `not needed`); `off (not needed)` que a regra do Linux diz que a CPU não
tem a vulnerabilidade; `off (not implemented)` que a CPU a tem e o kernel não a mitiga, o que é o
caso de v2, ssb, mds, bhb e kpti em CPU vulnerável (cada um com uma dívida, DEBT-0100 a DEBT-0104);
`off (untested)` que está ligada e nenhum ambiente a exercita. **O que ela não promete:** que a
detecção esteja completa (as listas de CPU são parte das do Linux,
[DEBT-0106](../tech-debt/0106-listas-de-cpu-do-linux-so-em-parte.md)) nem que a QEMU mostre o que o
metal mostraria (a TCG não anuncia `SPEC_CTRL` nem `ARCH_CAPABILITIES`). A regra de cada
vulnerabilidade, a fonte e o formato inteiro estão no [SDD 17](17_processos_isolamento_e_syscalls.md#mitigações-de-cpu).

## Endereços do kernel no log

Quem lê o log não aprende onde o KASLR pôs a imagem, o mapa direto nem o heap. A regra vale
para o boot, o relatório de `fault` e o pânico, na serial e na tela.

| O quê | Como aparece |
| --- | --- |
| Base da imagem | Não aparece. `[rustos] kaslr: on, image slid by up to N bits (4 KiB steps in 2 GiB)`, ou `kaslr: off` se a base é a do linker script |
| Mapa direto | Só o tamanho (`vm: direct map in N 2M and M 4K pages`) |
| `pc` do `fault`, endereço do page fault, ponteiro do heap | `Addr`: `kernel+0x1234` na imagem (confere com `nm` no ELF), `direct+0x40` no mapa direto, `mmio+…` e `stack+…` nas janelas, `<kernel>` em qualquer outro endereço da metade alta; o resto (metade baixa, `-1`) sai como veio |
| Mensagem do pânico e do `kprintln!` | Passa por um filtro (`Scrubber`, `core/hal::redact`) que troca todo literal `0x…` de endereço do kernel pela mesma forma. É a rede para `{:p}`, `{:#x}` e o `Debug` de `VirtAddr`; o código novo usa `Addr` (o `Debug` dele escreve como o `Display`) |
| Saída do shell | Sem filtro: é o que o usuário pediu (`random` imprime um `u64` qualquer) |

**O que o filtro não pega.** Só o literal com prefixo `0x`, de até 32 dígitos (zeros à esquerda valem; mais de 16 dígitos
significativos não é `u64` e sai como veio). Escapam o hexadecimal sem prefixo (`{:x}`), o decimal, o binário e o octal, e o
endereço que um tipo escreva por conta própria sem `Addr` nem `0x`; por isso o tipo que carrega endereço escreve em
hexadecimal com prefixo (`Violation`, `VirtAddr`) e o review confere o resto. Decimal de 19 dígitos ou mais acima de `i64::MAX` não é tratado.
`HIGH_HALF` (`0xffff_8000_0000_0000`) supõe paginação de 4 níveis, como o resto do kernel. `direct+0x…` é o endereço
**físico** do objeto do heap (o mapa direto soma um deslocamento fixo), não um deslocamento do KASLR. Custo do filtro, medido
no host em `release` (`cargo test --release -p rustos-hal -- --ignored --nocapture`, uma linha típica do boot): 94 a 97 ns e 11
chamadas a `write_str` por linha, contra 110 ns e 15 antes de o `0` só abrir candidato quando o `x` vem junto no mesmo pedaço
(a linha tem 9 pedaços; no kernel, cada chamada vira um desenho da tela).

**O teto de entropia vem do Limine, não do kernel.** Em `common/lib/elf.c` (`elf64_load`, lido em
2026-11-17) o sorteio é `rand32() & ~(max_align - 1)` somado à base do linker, e o Limine tenta
de novo se a imagem passa do topo: a base cai nos 2 GiB do topo, em passos do maior `p_align` do ELF
(4 KiB aqui, o que as medidas do [SDD 13](13_memoria_virtual.md) confirmam). Com a imagem de
cerca de 180 KiB o teto é `floor(log2((2 GiB - imagem) / 4 KiB))`, 18 bits. É um teto do desenho: o
kernel não vê o sorteio, só o resultado, e `slide == 0` (um boot em 2^18) sai como `kaslr: off`.

**O que o Linux faz.** `%p` imprime um hash com chave por boot (`ptr_to_id` em `lib/vsprintf.c`; `no_hash_pointers`
na linha de comando o desliga, com um aviso) e `kptr_restrict` zera `%pK` para quem não tem
`CAP_SYSLOG`. No `arm64`, `kaslr_init` só diz `KASLR enabled`; no x86_64, `dump_kernel_offset` imprime `Kernel Offset: 0x…` no pânico
(`arch/x86/kernel/setup.c`). O RustOS escolheu o deslocamento dentro da região em vez do hash: dá para depurar com o `nm` do
ELF sem revelar a base, e o `Addr` de dois logs do mesmo boot continua comparável.

**Depuração.** `RUSTOS_FEATURES=debug-addrs make run` (feature `debug-addrs` do kernel) imprime
o endereço cru, o `kernel base: 0x… (physical 0x…)` e desliga o filtro. Fora do padrão: entrega o KASLR.
O `make smoke` com ela só confere que a base não é a do linker script.

**Como se verifica.** O `make smoke` exige a linha `kaslr: on` e reprova qualquer literal `0x…` entre
`0xffff_8000_0000_0000` e `0xffff_ffff_ffff_0000` do primeiro `[rustos]` em diante, o corpo do pânico e a saída do shell incluídos (`xtask/src/leak.rs`, com
teste); o `make smoke-panic` roda `mem`, `ps` e `fetch` e passa a saída pelo mesmo conferidor. O corte de cima deixa passar `-1`/`u64::MAX`, e a de baixo, endereço de usuário e máscara.

## Aleatório do kernel

O gerador mora em `core/crypto` (`rustos-crypto`), um crate puro: não sabe de
hardware. A semente (RDSEED, RNDR) e o estado global são do `kernel/`, que os liga
(seção abaixo). Quatro módulos, sem alocação, ponto flutuante nem crate externo:

| Módulo | O que é | Spec |
| --- | --- | --- |
| `chacha20` | Bloco de 64 bytes (chave de 32, contador `u32`, nonce de 12) e keystream com XOR | RFC 8439, seções 2.3 e 2.4 |
| `blake2b` | Hash de 1 a 64 bytes, chave opcional de até 64, entrada em pedaços | RFC 7693 |
| `rng` | `Rng`: o gerador, com `new`, `reseed`, `fill_bytes`, `next_u64` e `bytes_since_seed` | este capítulo |
| `seed` | `gather_jitter` (semente de reserva pelo contador) e `hardware_output_plausible` | este capítulo |

`blake2b` existe também para o Argon2id da fase 7, que usa BLAKE2b como função
interna (RFC 9106, não lido nesta rodada).

### O desenho do `Rng`

O estado é **uma chave ChaCha20 de 32 bytes** e um contador de 8 bytes (`size_of`
40). O nonce é fixo em zero: cada chave serve a um pedido só e é trocada antes de
ele terminar, então o par (chave, nonce, contador) nunca se repete.

| Operação | O que faz |
| --- | --- |
| `fill_bytes(out)` | Um pedido: o bloco 0 com a chave atual dá a **chave nova** (primeiros 32 bytes) e os primeiros 32 bytes da saída; os blocos 1 em diante, ainda com a chave velha, dão o resto. A chave velha é substituída e o bloco 0 é zerado |
| `next_u64()` | `fill_bytes` de 8 bytes, em little-endian: um bloco por chamada |
| `reseed(entropy)` | `chave = BLAKE2b-256(chave = chave antiga, mensagem = entropy)` e zera o contador. A entrada nunca apaga o que a chave já sabia |
| `new(seed)` | `reseed(seed)` sobre uma chave de zeros (`unseeded()`); a semente tem qualquer tamanho, inclusive vazia (então é tão fraca quanto ela) |
| `unseeded()` | Chave de zeros, previsível: só para semear no lugar com `reseed`, sem a chave passar por uma cópia na pilha (o `random::init` do kernel) |
| `bytes_since_seed()` | Bytes entregues desde o último `new` ou `reseed`: o kernel decide por ele quando reabastecer |

Duas propriedades que o desenho dá e os testes conferem:

- **Sigilo para trás.** Depois de um pedido, a chave que gerou a saída já foi
  sobrescrita; quem lê o estado não reconstrói a saída anterior. É a ideia do
  Bernstein, no lugar do passo extra do DRBG do NIST.
- **Saída diferente por semente.** Uma semente com 1 bit trocado muda cerca de
  metade dos bits da saída.

### Decisões do desenho

| Decisão | Por quê |
| --- | --- |
| **A chave troca por pedido, não por bloco.** O Linux troca por bloco e entrega só 32 de cada 64 bytes (`crng_fast_key_erasure`); o Bernstein gera várias vezes o bloco e troca uma vez | Pedido grande (zerar página, 4 KiB) roda à velocidade do ChaCha20 em vez de à metade; pedido de até 32 bytes custa um bloco |
| **Pedidos em pedaços não dão a mesma saída que um pedido só**, porque a chave troca a cada chamada. Só os primeiros 32 bytes de qualquer pedido independem do tamanho dele | É o preço do sigilo para trás; quem precisa de reprodutibilidade fixa o plano de tamanhos (os testes fazem isso) |
| **Pedido acima de `MAX_REQUEST` (1 MiB) é partido em pedidos de 1 MiB**, cada um com a chave nova | O contador de 32 bits nunca chega ao fim (`xor_keystream` recusa passar de 2^32 blocos com `CounterOverflow`, em vez de repetir keystream) |
| **`reseed` usa BLAKE2b com chave, não o hash da entrada** | A chave antiga entra na conta: entropia ruim não piora o que já era bom, e entropia boa conserta o que era ruim. Como o Linux, que mistura as fontes com BLAKE2s e deriva a chave da saída (`extract_entropy`) |
| **Sem contabilidade de entropia** no crate | Quanto vale cada fonte é política do kernel; o crate só conta bytes entregues |
| **Sem `Clone`, `Copy` nem `Debug` derivado.** `Debug` escreve `<redacted>` ([guidelines 09](../guidelines/09_seguranca.md)); `Drop` zera a chave | Duas cópias do estado entregariam a mesma saída duas vezes |
| **O estado do ChaCha20 também não é `Copy`**: `chacha20::Words` (16 palavras, privado) tem `Drop` que o zera. O `[u32; 16]` de antes era `Copy`, e cada passagem por valor deixava uma cópia | Tipo `Copy` multiplica as cópias sem o código dizer; sem `Copy`, o compilador move em vez de copiar e o `Drop` roda uma vez por valor |
| **O estado é montado e consumido no lugar**: `Words::load` escreve num `Words` de quem chama, `permute(&state, &mut out)` trabalha em `out`, e `keyed_256` e `block_into` escrevem no buffer de quem chama | Devolver `[T; N]` por valor, em `opt-level` baixo, deixa o original no quadro de quem montou: no `release-small` o autoteste de resíduo achava uma cópia a mais |
| **Zerar usa `core::hint::black_box` e `compiler_fence`**, sem `unsafe` | Melhor esforço: a documentação do `std` não garante o `black_box`, e ele não desfaz cópias feitas antes do `wipe` (o `memcpy` de um move deixa a chave para trás). O `zeroize` usa `write_volatile`, que exige `unsafe`, proibido em `core/` (AGENTS, caso 5). Sem `unsafe`, é o melhor possível; o autoteste de pilha no alvo compensa |
| **ARX puro, sem tabela indexada por segredo**: o ChaCha20 e o BLAKE2b só somam, rodam e fazem XOR. O índice da `SIGMA` do BLAKE2b é público | Sem acesso à memória nem desvio que dependam da chave |

### O que o kernel faz

Está em `kernel/src/random.rs`, ligado no `kmain` depois das interrupções (o relógio
precisa estar de pé para a linha `random bench:`).

1. **Hardware, por uma porta do `hal`** (`HardwareEntropy::fill`, que devolve o nome da fonte), que o
   `arch/` implementa. `fill_from_words` (no `hal`, com teste) cuida da repetição
   limitada por palavra e da ordem little-endian.
   - **x86_64** (`X86Entropy`): o CPUID diz o que há (`RDRAND` é `CPUID.1:ECX[30]`, `RDSEED`
     é `CPUID.7.0:EBX[18]`). Usa `RDSEED`, com 64 tentativas por palavra (`pause` entre elas),
     e `RDRAND`, com 10 (as do Linux), se `RDSEED` esgotar ou não existir. Sucesso é `CF = 1`.
   - **aarch64** (`Aarch64Entropy`): `ID_AA64ISAR0_EL1.RNDR` (bits 63:60) diferente de zero
     anuncia as duas. Usa `RNDRRS` (semeado de novo a cada leitura) e `RNDR`, com 16
     tentativas por palavra. Sucesso deixa `NZCV = 0b0000`; falha, `0b0100`.
   - **Conferência:** 64 bytes de hardware passam por `hardware_output_plausible`
     (5 mudanças em 7 palavras seguidas, o teste do `x86_init_rdrand` do Linux); saída
     travada vale como "sem hardware".
2. **Jitter e marcos, sempre**, mesmo com hardware ("mais fonte não atrapalha"):
   `gather_jitter` (`core/crypto/src/seed.rs`, puro, testado com contador falso) lê o
   contador (`rdtsc`, `CNTVCT_EL0`) 512 vezes com um trabalho curto entre as leituras e
   devolve o BLAKE2b-256 delas. Os marcos do boot (ciclos na entrada do `kmain` e agora,
   os três tempos do bootloader, o relógio e os frames livres) entram por outro `reseed`.
3. **Fonte relatada:** `[rustos] random: rdseed` (ou `rdrand`, `rndrrs`, `rndr`) ou
   `[rustos] random: jitter (weak)`. O nome é o que de fato deu as palavras:
   `HardwareEntropy::fill` devolve o OU das instruções usadas, e quando o `RDSEED`
   falhou e o `RDRAND` completou a semente sai `rdseed+rdrand` (`rndrrs+rndr` no
   aarch64). Fraco porque nada mede quantos bits o jitter vale;
   no aarch64 sob HVF o contador anda a 24 MHz e cada amostra traz pouco ruído. A
   fraqueza é a [DEBT-0019](../tech-debt/0019-aleatorio-sem-fonte-de-hardware-fraco.md).
4. **Estado:** `static RNG: IrqLock<Option<Rng>>`. O comando `random` do shell chama
   `random::next_u64` e imprime em hexadecimal.
5. **Reposição:** quando `bytes_since_seed` passa de `RESEED_BYTES` (4 MiB), o
   `next_u64` repete a mistura do item 1 e 2. O número vem da medição (SDD 07). Não há
   reposição por tempo.
6. **Higiene:** o buffer de 64 bytes do hardware e os 32 do jitter são zerados com
   `rustos_crypto::wipe` ao sair de `replenish`. O gerador é semeado no lugar, no `static`
   (`Rng::unseeded` e `reseed`), sem passar por uma cópia do `Rng` na pilha.
7. **Resíduo na pilha, medido** (`random::stack_residue`, a linha `[rustos] key residue:`
   do boot). Enche com `0x5A` os 2 560 bytes abaixo do quadro, semeia um gerador do heap com
   a chave conhecida de `Rng::new(&[])`, atende um pedido e conta, nos 2 KiB abaixo de um
   local do quadro, os trechos de 16 bytes iguais às constantes do ChaCha20 (`expand
   32-byte k`, que antecedem a chave no estado) e à chave antiga. Números por ISA e perfil:
   SDD 07; depois, 0 e 0 nos seis.
   - **É heurística que depende do código gerado**: imprime `FAILED` e o boot segue; o
     `make smoke` reprova pelo `FAILED`.
   - **Roda com a IRQ mascarada.** A IRQ cai na pilha da tarefa e empilha o quadro abaixo
     do `sp`, exatamente na faixa varrida: com ela ligada, a sonda dava `0 of 4` ao acaso
     (smoke do x86_64 no CI, TCG lento) e um resíduo real podia sumir antes da leitura.
   - **Sonda de sensibilidade.** Uma segunda passada planta 4 estados falsos no quadro do
     primeiro callee e exige achá-los (`probe 4 of 4`). Sem ela, a varredura tinha uma zona
     cega nos cerca de 64 bytes logo abaixo do `sp` (o quadro da própria varredura caía
     ali): com `count_windows` fora de linha a sonda dá `0 of 4` no x86_64 `release`.
     **Limite:** a sonda só pega essa regressão onde o quadro da varredura cai sobre o
     plantio. Com `count_windows` fora de linha, ela reprova no x86_64 `dev` e `release`,
     mas passa no aarch64 (os três perfis) e no x86_64 `release-small`. Não dá falso
     "achou": exige exatamente 4, e uma passada sem plantio dá `0 of 4`.
   - **O que cobre:** `Words` e o `Rng` semeado com zero (a chave de `Rng::new(&[])`),
     não a chave real de `replenish` nem o `gather_jitter`.
   - **O que não vê:** o estado de trabalho no meio das rodadas, sem padrão reconhecível.
     No aarch64 o `permute` não o derrama (só salva registradores do chamador); no x86_64
     `release` guarda poucas palavras num quadro de 40 bytes, e `Rng::new` e
     `gather_jitter` devolvem por valor. A limpeza da pilha do kernel na volta ao usuário (fatia 4a,
     [SDD 17](17_processos_isolamento_e_syscalls.md)) cobre esse resíduo.
   - **O `v` do BLAKE2b** (`compress`) passou a ser zerado como o `m`: dava para reconstruir
     o `h` anterior a partir dele. Custo no host: 4 MiB de 1 595–1 606 para 1 554–1 578 MB/s
     (-2%), BLAKE2b-256 com chave de 191 para 200–202 ns (+5%), `reseed` de 164–167 para
     172–173 ns. O `v` não tem padrão que a varredura reconheça: tirar o `wipe(&mut v)` não
     muda a linha do boot. Quem confere é a desmontagem (`memset` de 128 B no aarch64,
     `rep stosl` ou `movq $0` no x86_64), lida na revisão do PR.

O que se observou rodando, com `-cpu max` salvo nota: x86_64 em TCG, `rdseed`; x86_64
com `-cpu Haswell` (sem `RDSEED`), `rdrand`; x86_64 com `-cpu qemu64` (sem os dois),
`jitter (weak)`; aarch64 em TCG, `rndrrs`; aarch64 sob HVF no Apple M5, `jitter (weak)`,
ou seja, a CPU não anuncia `RNDR`.

### Vetores de teste

Os testes rodam com `make test`. Os vetores vêm dos documentos, não da memória:

| Algoritmo | Vetores | Origem |
| --- | --- | --- |
| ChaCha20 | Rodada de quarto: seções 2.1.1 e 2.2.1; bloco: 2.3.2; cifra: 2.4.2; blocos do apêndice A.1 (cinco vetores); cifra do A.2 (três vetores) | RFC 8439, texto do `rfc-editor.org`, extraído por script para `core/crypto/tests/chacha20_rfc8439.rs` |
| BLAKE2b | "abc" do apêndice A; 256 entradas sem chave e 256 com chave de 64 bytes (entradas de 0 a 255 bytes) | RFC 7693; `testvectors/blake2-kat.json` do repositório de referência (commit `ed1974e`, 2023-02-12), conferido contra o `blake2b-kat.txt` do mesmo repositório |
| BLAKE2b, saídas de 1, 20, 32 e 48 bytes e chaves de 17 e 32 bytes | Casos que o KAT oficial não cobre | `hashlib.blake2b` do CPython; o caso "abc" de 64 bytes dele bate com o RFC |
| `Rng` | Saída fixa para as sementes `rustos` e vazia, com `reseed` no meio | Implementação independente em Python puro (ChaCha20 do RFC 8439 e `hashlib.blake2b`), que segue só este capítulo |

Mais os testes de propriedade do `Rng`: determinismo, semente diferente dá saída
diferente, a chave muda a cada pedido, pedidos em pedaços (a diferença está
documentada acima), `reseed` determinístico que mantém o estado, `Debug` sem a
chave, e contagem de bits (64 KiB, janela de 6 desvios) e de valores de byte. A
estatística é sanidade, **não prova de qualidade**.

### Fontes lidas

| Fonte | Lida em 2026-11-17 | Usada para |
| --- | --- | --- |
| [RFC 8439](https://www.rfc-editor.org/rfc/rfc8439.html), seções 2.1 a 2.4 e apêndice A | texto inteiro baixado | Constantes, estado, rodadas, contador de 32 bits, vetores |
| [RFC 7693](https://www.rfc-editor.org/rfc/rfc7693.html) | texto inteiro baixado | IV, `SIGMA`, função G, bloco de parâmetros, chave no primeiro bloco, vetor do apêndice A |
| [Bernstein, *Fast-key-erasure random-number generators*](https://blog.cr.yp.to/20170723-random.html), 2017-07-23 | página baixada | "Immediately overwrite the key k with the first two blocks B0, B1. Use the other blocks B2, ..., B47 as 736 bytes of RNG output [...] Start over with the new key" |
| [`drivers/char/random.c`](https://github.com/torvalds/linux/blob/master/drivers/char/random.c), Linux | `master` lido por ferramenta de busca que resume a página, não o arquivo cru | Só referência: BLAKE2s acumula e deriva a chave (`extract_entropy`), ChaCha20 expande, `crng_fast_key_erasure` troca a chave por metade do bloco |
| [`arch/x86/include/asm/archrandom.h`](https://github.com/torvalds/linux/blob/master/arch/x86/include/asm/archrandom.h), Linux | arquivo cru baixado | `RDRAND_RETRY_LOOPS` = 10; `RDRAND` e `RDSEED` devolvem sucesso na flag `CF` (`"=@ccc"`) |
| [`arch/x86/include/asm/cpufeatures.h`](https://github.com/torvalds/linux/blob/master/arch/x86/include/asm/cpufeatures.h), Linux | arquivo cru baixado | `X86_FEATURE_RDRAND` = palavra 4, bit 30 (`CPUID.1:ECX[30]`); `X86_FEATURE_RDSEED` = palavra 9, bit 18 (`CPUID.7.0:EBX[18]`) |
| [`arch/x86/kernel/cpu/rdrand.c`](https://github.com/torvalds/linux/blob/master/arch/x86/kernel/cpu/rdrand.c), Linux | arquivo cru baixado | Teste de sanidade: 8 amostras, no mínimo 5 mudanças (defeitos de CPU já devolveram o mesmo valor sempre) |
| [`arch/arm64/include/asm/archrandom.h`](https://github.com/torvalds/linux/blob/master/arch/arm64/include/asm/archrandom.h) e [`sysreg.h`](https://github.com/torvalds/linux/blob/master/arch/arm64/include/asm/sysreg.h), Linux | arquivos crus baixados | `RNDR` = `S3_3_C2_C4_0`, `RNDRRS` = `S3_3_C2_C4_1`; `NZCV` = `0b0000` em sucesso e `0b0100` em falha; o campo `RNDR` de `ID_AA64ISAR0_EL1` está nos bits 63:60 |
| [`arch/arm64/tools/sysreg`](https://github.com/torvalds/linux/blob/master/arch/arm64/tools/sysreg), Linux | arquivo cru baixado | `ID_AA64ISAR0_EL1`: `63:60 RNDR`, `0b0000 NI`, `0b0001 IMP` |
| [`target/arm/helper.c`](https://github.com/qemu/qemu/blob/master/target/arm/helper.c) (`rndr_readfn`), QEMU | arquivo cru baixado | Sucesso deixa `NZCV = 0000`; falha, `0100` |

O Intel SDM, o Arm ARM e o guia de DRNG da Intel não foram lidos: as tentativas por palavra do `RDSEED` (64) e do `RNDRRS` (16) são escolha do RustOS, sem fonte, e os testes rodam em QEMU, não em metal. O que cada
instrução faz ao falhar vem do Linux e da QEMU.

Não há dependência de nenhum desses: o código é do RustOS.

## Decisões

Estão na lista fechada do `AGENTS.md` (casos 2, 4 e 6): quem decide é o dono do
projeto, e a decisão fica registrada aqui.

### Decidida: modelo de controle de acesso A

Decidido pelo dono do projeto em 2026-11-17, no PR #3:

- **No núcleo:** só handles com direitos que só diminuem, no modelo Fuchsia/seL4,
  sem autoridade ambiente.
- **Na camada da ABI Linux (fase 5):** uid, gid, grupos e capabilities(7) por
  processo, traduzidos para esses direitos. É o desenho do Asterinas e do gVisor.

| Opção | Como é | Por que não |
| --- | --- | --- |
| **A — Capacidades por dentro, Unix na ABI Linux** ✅ | Escolhida | — |
| B — Unix em tudo | uid/gid e bits de modo no núcleo, como o Linux | Autoridade ambiente (`root` pode tudo); endurecer depois exige LSM por cima |
| C — Só capacidades | Sem uid/gid, nem na ABI | Quebra programa Linux que consulta uid ou permissão; contradiz a meta da fase 5 |

Consequência para o código: nenhuma API do núcleo recebe uid. Quem precisa de
permissão recebe um handle; a tradução de uid para direito mora no adaptador da
ABI Linux.

### Aberta: boot verificado (casos 2 e 4)

Secure Boot exige assinar o Limine e o kernel com uma chave registrada no
firmware, e measured boot exige TPM 2.0. As duas coisas são binário ou plataforma
nova. A decisão é tomada antes de a fase 12 começar.

### Aberta: formato da raiz e criptografia de disco (caso 6)

Estão no SDD 08: o formato da raiz antes da fase 6, a criptografia de disco antes
da fase 8.
