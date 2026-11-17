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
| **Barato e difícil de pôr depois entra cedo** | W^X, NX, SMEP/SMAP, PAN e páginas de guarda entram nas fases 1 e 2, antes de existir usuário |
| **Caro entra medido** | Mitigação com custo (KPTI, retpoline, MTE) entra com o número no [SDD 07](07_desempenho_e_recursos.md) e liga só onde a CPU precisa |
| **Padrão seguro** | Rede nega o que entra por padrão; processo nasce sem handle nenhum; segredo nunca vai para log nem serial |

## Modelo de ameaça

| Atacante | O que pode fazer | Defesa | Fase |
| --- | --- | --- | --- |
| Programa de usuário malicioso | Ler ou escrever memória de outro processo ou do kernel, escalar privilégio, esgotar recurso | Espaço de endereçamento por processo, SMEP/SMAP/PAN, handles com direitos, grupos de recursos | 4, 9 |
| Binário Linux com bug explorável | Estouro de buffer, ROP | ASLR, NX, CET/PAC/BTI, filtro de syscalls | 4, 12 |
| Entrada de rede | Pacote malformado, inundação | Pilha TCP/IP em Rust seguro, validação na fronteira, filtro que nega por padrão | 10 |
| Disco ou imagem malformada | Sistema de arquivos corrompido de propósito | Parser em `core/` sem `unsafe`, com fuzz | 6 |
| Dispositivo malicioso (DMA) | Escrever na memória do kernel por DMA | IOMMU (VT-d, SMMU) | depois de 6, sem fase ainda |
| Canal lateral de CPU | Meltdown, Spectre, MDS | Mitigações por CPU, relatadas no boot | 12 |
| Acesso físico, firmware adulterado | Trocar kernel ou bootloader; ler o disco de uma máquina roubada | Boot verificado (Secure Boot, TPM); criptografia de disco no instalador | 8, 12, decisões abertas |

Fora de escopo por enquanto: ataque físico a memória (cold boot), canal lateral de
energia e firmware malicioso abaixo do UEFI.

## Mecanismos, por fase

| Mecanismo | Protege contra | Fase | Custo publicado | Fonte |
| --- | --- | --- | --- | --- |
| WP, SMEP, SMAP, UMIP (x86_64) | Kernel executar ou ler página de usuário por engano; `sgdt`/`sidt` vazando endereço | 1 | Desprezível | Intel SDM vol. 3 (não lido nesta rodada) |
| PAN, PXN/UXN (aarch64) | O mesmo, no ARM | 1 | Desprezível | Arm ARM (não lido nesta rodada) |
| Pilhas de exceção com guarda (IST) | Estouro de pilha virar corrupção silenciosa | 1 | 1 página por pilha | [Linux kernel stacks](https://docs.kernel.org/arch/x86/kernel-stacks.html) |
| W^X no kernel, NX em dado | Injetar e executar código no kernel | 2 | Nenhum em execução | [kernel self-protection](https://docs.kernel.org/security/self-protection.html) |
| Aleatório do kernel (RDSEED/RNDR → ChaCha20) | Previsibilidade de endereço, chave e canário | 2 | A medir | [Fortuna](https://www.schneier.com/academic/fortuna/) |
| KASLR | Endereço fixo do kernel facilitando ROP | 2 | Kernel relocável (tamanho a medir) | [x86_64 mm](https://docs.kernel.org/arch/x86/x86_64/mm.html) |
| Zerar memória ao liberar | Vazar dado de um processo para outro | 2 | A medir | [kernel-parameters](https://www.kernel.org/doc/html/latest/admin-guide/kernel-parameters.html) (`init_on_free`) |
| Espaço de endereçamento por processo, ASLR de usuário | Um processo ler outro | 4 | Troca de tabela por troca de contexto | [self-protection](https://docs.kernel.org/security/self-protection.html) |
| Handles com direitos | Autoridade ambiente | 4 | Uma consulta por operação | [Fuchsia handles](https://fuchsia.dev/fuchsia-src/concepts/kernel/handles) |
| Credenciais Linux (uid/gid, capabilities(7)) | Programa Linux esperar o modelo Unix | 5 | — | [capabilities(7)](https://man7.org/linux/man-pages/man7/capabilities.7.html) |
| Permissões de arquivo (dono, grupo, modo) | Usuário ler arquivo de outro | 6 | Uma comparação por `open` | POSIX |
| Criptografia de disco (opcional no instalador) | Ler o disco fora da máquina | 8 | A medir: custo por bloco de E/S | decisão aberta (SDD 08) |
| Argon2id para senha | Quebra de senha vazada por força bruta | 7 | ~64 MiB e t=3 por login (perfil de memória limitada) | [RFC 9106](https://www.rfc-editor.org/rfc/rfc9106.html) |
| Grupos de recursos (CPU, memória, processos, E/S) | Um processo esgotar a máquina | 9 | A medir | [cgroup v2](https://docs.kernel.org/admin-guide/cgroup-v2.html) |
| KPTI (só CPU vulnerável a Meltdown) | Meltdown | 12 | ~100 ciclos por troca de CR3; sem PCID esvazia a TLB | [Linux PTI](https://docs.kernel.org/arch/x86/pti.html) |
| Retpoline/eIBRS, IBPB, STIBP | Spectre v2 | 12 | Sem número na fonte | [Linux spectre](https://docs.kernel.org/admin-guide/hw-vuln/spectre.html) |
| Limpeza de buffers (VERW) | MDS | 12 | SMT desligado é o mais caro | [Linux MDS](https://docs.kernel.org/admin-guide/hw-vuln/mds.html) |
| CET shadow stack e IBT | ROP e JOP | 12 | — | [Linux shstk](https://docs.kernel.org/arch/x86/shstk.html) |
| PAC, BTI | ROP e JOP no ARM | 12 | — | [Linux PAC](https://www.kernel.org/doc/html/latest/arch/arm64/pointer-authentication.html) |
| MTE | Uso depois de liberar, estouro de heap | 12 | Sem número na fonte | [Linux MTE](https://www.kernel.org/doc/html/latest/arch/arm64/memory-tagging-extension.html) |
| Filtro de syscalls por processo | Programa comprometido chamar o que não precisa | 12 | Um filtro por syscall | [seccomp](https://docs.kernel.org/userspace-api/seccomp_filter.html), [pledge](https://man.openbsd.org/pledge) |
| Boot verificado e medido | Kernel ou bootloader trocados | 12 | Tempo de boot a medir | [TPM 2.0](https://trustedcomputinggroup.org/resource/tpm-library-specification/) |

Levantamento de novembro de 2026. O que aparece como "não lido nesta rodada" vem
de fonte secundária e precisa da leitura do manual antes de virar código.

## Decisões abertas

Estas decisões estão na lista fechada do `AGENTS.md` (casos 2, 4 e 6): o agente
não as toma.

### 1. Modelo de controle de acesso — decidido: A

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

### 2. Boot verificado (casos 2 e 4)

Secure Boot exige assinar o Limine e o kernel com uma chave registrada no
firmware, e measured boot exige TPM 2.0. As duas coisas são binário ou plataforma
nova. A decisão fica para a fase 12, junto com a DEBT-0001 (bootloader próprio).
