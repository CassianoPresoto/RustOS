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
| Aleatório do kernel: semente de RDSEED (x86_64) ou RNDR (aarch64), gerador ChaCha20 | Previsibilidade de endereço, chave e canário | 2 | A medir | [RFC 8439](https://www.rfc-editor.org/rfc/rfc8439.html) (ChaCha20); RDSEED e RNDR não lidos nesta rodada. CPU sem os dois (RNDR é ARMv8.5) precisa de outra semente: decisão da fase 2 |
| KASLR | Endereço fixo do kernel facilitando ROP | 2 | Kernel relocável (tamanho a medir) | [x86_64 mm](https://docs.kernel.org/arch/x86/x86_64/mm.html) |
| Zerar memória ao liberar | Vazar dado de um processo para outro | 2 | A medir | [kernel-parameters](https://www.kernel.org/doc/html/latest/admin-guide/kernel-parameters.html) (`init_on_free`) |
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
