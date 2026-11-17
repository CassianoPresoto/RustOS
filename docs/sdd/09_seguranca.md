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
| Canal lateral de CPU | Meltdown, Spectre, MDS | Mitigações por CPU, relatadas no boot | 4 |
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
| Pilhas de exceção separadas | Exceção com a pilha corrompida derrubar a máquina sem relatório | 1 | 1 página por pilha, mais 1 de guarda | x86_64: IST, em [Linux kernel stacks](https://docs.kernel.org/arch/x86/kernel-stacks.html). aarch64: o kernel roda em EL1t, com `SP_EL0`, e a exceção troca para `SP_EL1`, que aponta para uma pilha só de exceção (desenho do RustOS; Arm ARM não lido nesta rodada). A página de guarda abaixo de cada pilha é desenho do RustOS, a criar na fase 1 |
| PXN/UXN nas tabelas próprias (aarch64) | Kernel executar página de usuário; usuário executar página do kernel | 2 | Desprezível (não lido nesta rodada) | Arm ARM (não lido nesta rodada) |
| W^X no kernel, NX em dado | Injetar e executar código no kernel | 2 | A medir: pode exigir quebrar página grande e custar TLB | [kernel self-protection](https://docs.kernel.org/security/self-protection.html) |
| Aleatório do kernel: semente de RDSEED (x86_64) ou RNDR (aarch64), gerador ChaCha20 com *fast key erasure* (`core/crypto`) | Previsibilidade de endereço, chave e canário | 2 | Estado de 40 bytes; `next_u64` 67 ns e bloco de 4 KiB a 960 MB/s no host, escalar (Apple M5); no alvo, a medir ([SDD 07](07_desempenho_e_recursos.md)) | [RFC 8439](https://www.rfc-editor.org/rfc/rfc8439.html) (ChaCha20), [RFC 7693](https://www.rfc-editor.org/rfc/rfc7693.html) (BLAKE2b), [Bernstein, 2017](https://blog.cr.yp.to/20170723-random.html) (fast key erasure), [`random.c` do Linux](https://github.com/torvalds/linux/blob/master/drivers/char/random.c) (referência); RDSEED e RNDR não lidos nesta rodada. CPU sem os dois (RNDR é ARMv8.5) precisa de outra semente: decisão da fase 2 |
| KASLR | Endereço fixo do kernel facilitando ROP | 2 | Kernel relocável (tamanho a medir) | [x86_64 mm](https://docs.kernel.org/arch/x86/x86_64/mm.html) |
| Zerar memória ao liberar | Vazar dado de um processo para outro | 2 | Host, cache quente: cerca de 7 ns por KiB (16 B: +1 ns; 1 KiB: +9 ns; página: +27 ns); metal ou HVF ainda não medido (SDD 07). Política `KernelHeap::set_zero_on_free` | [kernel-parameters](https://www.kernel.org/doc/html/latest/admin-guide/kernel-parameters.html) (`init_on_free`) |
| Espaço de endereçamento por processo | Um processo ler outro | 4 | A medir: troca de tabela a cada troca de contexto | Desenho do RustOS (SDD 08, fase 4) |
| ASLR de usuário | Endereço fixo de pilha, heap e bibliotecas facilitando ROP | 4 | A medir | [sysctl kernel](https://docs.kernel.org/admin-guide/sysctl/kernel.html) (`randomize_va_space`) |
| Handles com direitos | Autoridade ambiente | 4 | A medir | [Fuchsia handles](https://fuchsia.dev/fuchsia-src/concepts/kernel/handles) |
| KPTI (só CPU vulnerável a Meltdown) | Meltdown | 4 | ~100 ciclos por troca de CR3; sem PCID, esvazia a TLB | [Linux PTI](https://docs.kernel.org/arch/x86/pti.html) |
| Retpoline/eIBRS, IBPB, STIBP | Spectre v2 | 4 | Sem número na fonte | [Linux spectre](https://docs.kernel.org/admin-guide/hw-vuln/spectre.html) |
| Limpeza de buffers da CPU na troca de privilégio | MDS | 4 | Desligar SMT tem "impacto significativo", sem número na fonte | [Linux MDS](https://docs.kernel.org/admin-guide/hw-vuln/mds.html) |
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

## Aleatório do kernel

O gerador mora em `core/crypto` (`rustos-crypto`), um crate puro: não sabe de
hardware. A semente (RDSEED, RNDR) e o estado global são do `kernel/`, que liga os
dois na fase 2. Três módulos, sem alocação, ponto flutuante nem crate externo:

| Módulo | O que é | Spec |
| --- | --- | --- |
| `chacha20` | Bloco de 64 bytes (chave de 32, contador `u32`, nonce de 12) e keystream com XOR | RFC 8439, seções 2.3 e 2.4 |
| `blake2b` | Hash de 1 a 64 bytes, chave opcional de até 64, entrada em pedaços | RFC 7693 |
| `rng` | `Rng`: o gerador, com `new`, `reseed`, `fill_bytes`, `next_u64` e `bytes_since_seed` | este capítulo |

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
| `new(seed)` | `reseed(seed)` sobre uma chave de zeros; a semente tem qualquer tamanho, inclusive vazia (então é tão fraca quanto ela) |
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
| **Zerar usa `core::hint::black_box`**, sem `unsafe` | Melhor esforço: o Rust não garante que escrita sem leitura sobreviva à otimização, e `write_volatile` exige `unsafe`, proibido em `core/` |
| **ARX puro, sem tabela indexada por segredo**: o ChaCha20 e o BLAKE2b só somam, rodam e fazem XOR. O índice da `SIGMA` do BLAKE2b é público | Sem acesso à memória nem desvio que dependam da chave |

### O que o kernel faz (fase 2)

Nada disto está no kernel ainda:

1. **Semente:** pelo menos 32 bytes de RDSEED (x86_64) ou RNDR (aarch64), por uma
   porta no `hal` que o `arch/` implementa. As duas instruções podem falhar e
   pedir nova tentativa (não lido nesta rodada); o que fazer sem elas é decisão da
   fase 2.
2. **Mistura:** além do hardware, o que o boot tiver de impredizível (contadores de
   ciclo, tempos de dispositivo) entra por `reseed`. Mais fonte não atrapalha.
3. **Estado:** um `SpinLock<Option<Rng>>` no `kernel/`. Quem chama de tratador de
   interrupção usa o lock com interrupções mascaradas.
4. **Reposição:** quando `bytes_since_seed` passar de um limite, ou de tempos em
   tempos, o kernel faz `reseed` com semente nova de hardware. O limite sai da
   medição, não de palpite.
5. **Higiene:** o buffer da semente é zerado com `rustos_crypto::wipe` depois do
   `new`.

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
