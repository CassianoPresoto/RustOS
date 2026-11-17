# SDD 08 — Roadmap

> **O que este capítulo responde:** em que ordem o sistema cresce, o que cada fase
> entrega (inclusive de segurança e de gerência de recursos) e o que marca o fim de
> cada uma.

Cada fase termina com `make pr` verde e um número novo no
[SDD 07](07_desempenho_e_recursos.md). A ordem segue dependência técnica: não há
escalonador sem interrupção de timer, nem espaço de usuário sem memória virtual,
nem programa sem um sistema de arquivos de onde carregá-lo, nem conta de usuário
sem disco onde guardá-la.

**Segurança não é uma fase.** Cada fase entrega os mecanismos que a tornam segura,
e o que é barato e difícil de pôr depois entra cedo (W^X, NX, SMEP/SMAP, PAN). O
modelo de ameaça e o porquê de cada mecanismo estão no [SDD 09](09_seguranca.md).

## A fila

| Fase | Entrega | Pronto quando |
| --- | --- | --- |
| **0 — Esqueleto** ✅ | Workspace hexagonal, boot pelo Limine em x86_64 e aarch64, alocador de frames, serial, xtask, gates, CI | `make smoke ARCH=all` passa |
| **0.5 — Tela e terminal** ✅ | Framebuffer, fonte própria, emulador VT/xterm, shell embutido, barra no topo | `fetch` desenhado na tela das duas ISAs (`make screenshot`) |
| **1 — Exceções, interrupções e tempo** ✅ | Vetores de exceção, controlador de interrupção, timer, relógio, entrada por interrupção, proteções de hardware ligadas | Page fault proposital vira relatório; tique contado; shell dorme em `hlt`/`wfi` |
| **2 — Memória virtual e heap** ✅ | Tabelas próprias com W^X, heap do kernel, aleatório do kernel, memória do bootloader recuperada | Escrita em `.text` falha; memória livre depois do boot medida |
| **3 — Tarefas, SMP e escalonador** | Troca de contexto com estado FPU/SIMD por tarefa, threads do kernel, fila por CPU, escalonador atrás de trait (rodízio, depois EEVDF), timer de um tiro, idle sem tique | Latência de troca medida em N núcleos, com e sem estado SIMD |
| **4 — Processos e isolamento** | Anel 3/EL0, syscall, carregador ELF, VFS em memória (initramfs, tmpfs), espaço de endereçamento por processo, handles com direitos, mitigações de CPU, SSE/AVX e NEON/SVE liberados ao usuário, página zero e COW | Syscall nula medida; processo que toca o kernel morre sem derrubar nada |
| **5 — ABI Linux e primeiros aplicativos** | Syscalls Linux traduzidas, credenciais Linux, musl estático, BusyBox | BusyBox roda sem recompilar e é medido contra o Linux; DOOM por software roda (marco 1 dos jogos); memória ociosa medida contra a linha de base ([Memória ociosa](#memória-ociosa-o-alvo-vem-de-uma-medida)) |
| **6 — Armazenamento persistente** | PCI Express, IOMMU, virtio-blk, NVMe, AHCI, page cache, sistema de arquivos da raiz com escrita, permissões em disco, ACPI/DTB, virtio-input e virtio-snd, readahead com janela crescente, LRU de gerações | Benchmark de E/S ao lado do Linux |
| **7 — Usuários e sessões** | Contas e grupos, login, Argon2id, elevação de privilégio por capacidade, auditoria | Dois usuários sem acesso aos arquivos um do outro |
| **8 — Instalação** | Imagem live de pendrive, teclado e mouse USB (xHCI), instalador em modo texto, GPT, partição EFI, entrada de boot UEFI, gamepad USB HID | Instalar num disco vazio da QEMU e subir dele sem a mídia live |
| **9 — Gerência de recursos e energia** | Grupos de recursos (CPU, memória, processos, E/S), limites por processo, falta de memória, contabilidade, idle states, PSI, `memory.high`, compressão LZ4 | Grupo limitado respeita o limite sob carga |
| **10 — Rede** | virtio-net, pilha TCP/IP própria, sockets da ABI Linux, DNS, filtro de pacotes | Vazão e latência TCP ao lado do Linux |
| **11 — Interface gráfica** | Compositor, janelas, mouse, virtio-gpu 2D com page flip e vsync, damage explícito, buffer sem cópia, backend SDL próprio, aplicativos gráficos | Um aplicativo gráfico com tempo de quadro medido |
| **12 — Endurecimento avançado** | CET, PAC/BTI, MTE, CFI, filtro de syscalls, MAC, boot verificado | Cada mecanismo com custo medido no SDD 07 |
| **13 — Jogos 3D** | Mesa com virgl ou Venus sobre a ABI Linux e o DRM do guest | Um programa GL ou Vulkan desenha com aceleração do host e o tempo de quadro é medido |

Jogos entram por uma escada de seis marcos, do DOOM por software (fase 5) ao 3D
acelerado (fase 13): ver [Jogos: a escada](#jogos-a-escada). Os dois pedaços de
código de terceiros que ela exige (SDL e Mesa) são decisões futuras, listadas em
[Decisões futuras](#decisões-futuras-casos-do-agentsmd).

## O que cada fase entrega

### 1 — Exceções, interrupções e tempo

- **Exceções:** GDT, TSS e IDT no x86_64, com pilhas separadas (IST) para double
  fault, NMI e machine check. `VBAR_EL1` e decodificação de `ESR_EL1`/`FAR_EL1` no
  aarch64.
- **Interrupções:**
  - Local APIC (x2APIC quando houver) e 8259 mascarado no x86_64;
  - GICv3 com `gic-version=3` fixo na QEMU, nas três acelerações, no aarch64.
- **Tempo:** timer one-shot do APIC calibrado (sob TCG o `-cpu max` não tem
  TSC-deadline) e o timer virtual (`CNTV`, INTID 27) no aarch64. Relógio
  monotônico, e `uptime` no shell.
- **Entrada por interrupção:** UART nas duas ISAs e teclado PS/2 no x86_64.
- **Base de memória e locks:** `PageMapper` do x86_64 e lock que mascara
  interrupção (`IrqLock`).
- **Segurança:**
  - ligar WP, SMEP, SMAP e UMIP no x86_64 e PAN no aarch64;
  - relatar na serial o que a CPU tem e o que ficou ligado, na linha
    `[rustos] protections:`, que o `make smoke` passa a exigir;
  - página de guarda em toda pilha de exceção.

### 2 — Memória virtual e heap

- **Tabelas próprias do kernel** ([SDD 13](13_memoria_virtual.md)):
  - `.text` só leitura e execução, `.rodata` só leitura, dados com NX;
  - mapa direto refeito (2 MiB onde dá, sem a imagem), framebuffer com o cache de vídeo,
    metade baixa vazia;
  - pilha própria de 64 KiB, e página de guarda abaixo de toda pilha do kernel;
  - W^X conferido no boot (`w^x: ok`) e por `fault text`.
- **Heap:** buddy allocator e `GlobalAlloc` para `alloc`, **ligados**: o buddy entrega
  os frames e o heap de slabs é o alocador global ([SDD 12](12_memoria_fisica_e_heap.md)).
  O cache por CPU espera a fase 3 (DEBT-0018).
- **Memória do bootloader recuperada** (devolvida ao buddy depois de copiar o que se lê
  do Limine e de trocar tabelas, pilha e GDT).
- **Aleatório do kernel:** semente de RDSEED/RNDR (jitter do contador quando a CPU não
  tem, DEBT-0019), gerador ChaCha20 e reposição por volume, **ligados**. É a base de
  KASLR, ASLR e canários ([SDD 09](09_seguranca.md)).
- **KASLR:** kernel relocável (PIE) com o Limine aplicando a relocação e sorteando a
  base e o mapa direto. Custo de 3 a 6 KiB medido, boot sem diferença mensurável.
- **Zerar ao liberar:** política de zerar memória devolvida, **ligada** no heap, com o
  custo medido ([SDD 07](07_desempenho_e_recursos.md)).

### 3 — Tarefas, SMP e escalonador

- **Tarefas:** troca de contexto em `arch/`, threads do kernel, `core/sched` ligado
  ao kernel.
- **Estado FPU/SIMD por tarefa, sempre salvo (eager).** O kernel continua
  compilado soft-float: o estado só é do código de usuário. O porquê do eager é o
  LazyFP ([SDD 09](09_seguranca.md)); o custo está em [SDD 07](07_desempenho_e_recursos.md).

  | | x86_64 | aarch64 |
  | --- | --- | --- |
  | Salva na troca | `XSAVEOPT` (formato padrão, pula o que não mudou); `XSAVES` só se o kernel passar a usar estado de supervisor | V0–V31, FPSR e FPCR (cerca de 520 bytes), sempre |
  | Tamanho da área | CPUID 0xD lido **uma vez** no boot e guardado (a consulta custa centenas de ciclos) | Fixo no FPSIMD; SVE e SME alocados no primeiro uso |
  | Estado grande | AMX fora do boot (fase 4) | SVE e SME por trap de primeiro uso, VL padrão pequeno |

- **Escalonador:** `trait Scheduler` (`enqueue`, `pick_next`, `tick`) em `core/sched`.
  - Rodízio primeiro, para provar a troca de contexto e a fila por CPU.
  - Depois um EEVDF pequeno. Por tarefa: `vruntime`, `lag`, `slice` e deadline
    virtual. Por fila: soma ponderada dos `vruntime` e soma dos pesos, que dão o
    `avg_vruntime` em O(1). Elegível é quem tem `lag >= 0`; entre as elegíveis
    roda a de menor deadline virtual.
  - Três regras do Linux entram com a política: `PLACE_LAG` (o lag sobrevive ao
    sono), `DELAY_DEQUEUE` (quem dorme com lag negativo fica na fila até pagá-lo) e
    `RUN_TO_PARITY` (o acordar não preempta antes do ponto de lag zero ou do fim
    do slice). Fontes e marcas em [SDD 07](07_desempenho_e_recursos.md).
  - FIFO de tempo real, com teto de 95% de CPU: o padrão do Linux
    (950000/1000000, [sched-deadline](https://docs.kernel.org/scheduler/sched-deadline.html)).
- **SMP:** outros núcleos pelo pedido MP do Limine. O BSP grava `extra_argument` e
  depois `goto_address` com store de release; cada núcleo lê com acquire.
  - Área por CPU por `GS` (x86_64) e `TPIDR_EL1` (aarch64): prática comum, sem
    fonte lida.
  - Fila por CPU, com pull de CPU ociosa e balanceamento periódico.
- **Tempo e energia:**
  - timer de um tiro com expiração absoluta: TSC-deadline quando a CPU tiver, `CNTV`
    no aarch64 (já em uso desde a fase 1);
  - idle sem tique (NO_HZ_IDLE). NO_HZ_FULL fica para depois: exige CPU de
    housekeeping e transições mais caras.
- **Medir** (cada número entra no [SDD 07](07_desempenho_e_recursos.md)): `lat_ctx`,
  pipe ping-pong pinado, jitter do acordar e tiques por segundo com a máquina
  ociosa.
- **Ordem dos PRs:**

  | Passo | Entrega |
  | --- | --- |
  | a | Troca de contexto e timer de um tiro na BSP |
  | b | Rodízio |
  | c | EEVDF |
  | d | SMP |
  | e | Classe de tempo real |
  | f | NO_HZ_IDLE |

### 4 — Processos e isolamento

- **Execução de usuário:** anel 3/EL0, entrada de syscall e carregador ELF.
- **Arquivos em memória:** VFS mínimo, initramfs (os módulos que o Limine
  carrega) e tmpfs. É de onde sai o primeiro programa; o disco vem na fase 6.
- **Isolamento de memória:** espaço de endereçamento por processo.
  - O kernel fica mapeado em todos eles, mas só com permissão de supervisor: o
    código de usuário não lê nem escreve nada dele (U/S no x86_64, AP no
    aarch64).
  - ASLR de usuário;
  - NX em pilha e heap;
  - cópia usuário↔kernel só por rotina que respeita SMAP/PAN.
- **Mitigações de CPU:** chegam junto com o primeiro código de usuário, porque
  mexem na entrada e na saída de syscall que esta fase desenha.
  - Relatório no boot do que a CPU tem de vulnerável e do que ficou ligado.
  - KPTI só em CPU vulnerável a Meltdown: o espaço do usuário mapeia só o
    trampolim de entrada.
  - Spectre v2 e MDS onde a CPU precisa, com custo medido.
- **Controle de acesso interno:** handles com direitos que só diminuem, no modelo
  Fuchsia/seL4. Nenhuma autoridade ambiente: um processo só acessa o que recebeu.
- **Ciclo de vida:** criar, terminar, esperar e sinais mínimos.
- **SIMD para o usuário:**
  - x86_64: `CR4.OSFXSR`, `OSXMMEXCPT` e `OSXSAVE`; `XCR0` com x87, SSE, YMM e
    AVX-512 quando a CPU tiver. AMX só por pedido explícito do processo
    (equivalente ao `ARCH_REQ_XCOMP_PERM`, interface em decisão futura), porque o
    buffer é grande;
  - aarch64: `CPACR_EL1` liberando NEON, e SVE e SME quando a CPU tiver.
- **`rdtsc` em modo usuário liberado:** `CR4.TSD = 0`, com opção por processo para
  bloquear (como o `PR_SET_TSC` do Linux). Jogos e benchmarks leem o contador;
  o risco está no [SDD 09](09_seguranca.md).
- **Memória do processo:**
  - página zero compartilhada para leitura de anônimo não tocado, e COW no fork;
  - overcommit estrito como padrão: a falta de memória aparece como erro na
    alocação, não como processo morto;
  - THP só por `madvise`, nunca por padrão.

### 5 — ABI Linux e primeiros aplicativos

- **Adaptador de entrada:** traduz syscalls Linux para os serviços do núcleo, no
  modelo do Asterinas.
- **Credenciais Linux:** uid, gid, grupos e capabilities(7), traduzidos para os
  direitos internos (modelo A, decidido no SDD 09).
- **Primeiros aplicativos:** musl estático, BusyBox (`sh`, coreutils, `vi`, `top`)
  carregado do initramfs, pipes, tabela de descritores e `/proc` mínimo.
- **Shell embutido:** vira console de recuperação.
- **O que o programa pergunta à máquina:** `AT_HWCAP` e `AT_HWCAP2` no auxv, a partir
  do CPUID (x86_64) e dos `ID_AA64*` (aarch64). É contrato com o mundo de fora
  (caso 6 do `AGENTS.md`): o formato exato é decisão futura, listada abaixo.
- **Tempo:** `clock_gettime(CLOCK_MONOTONIC)` como syscall comum e medida;
  o vDSO vem depois, com o número que o justifique.
- **Jogos:** o primeiro marco da escada, DOOM por software, fecha esta fase.
- **Memória ociosa:** a medição contra a linha de base fecha esta fase.

### 6 — Armazenamento persistente

- **Descoberta de hardware:**
  - PCI Express pelo ECAM, lido do ACPI (MCFG) ou do DTB;
  - endereços de dispositivo também por ACPI e DTB, no lugar de endereço fixo
    (DEBT-0002).
- **IOMMU (VT-d, SMMU):** liga antes do primeiro driver com DMA rodar em metal.
  Um dispositivo só faz DMA na memória que o driver entregou a ele.
- **Disco:** virtio-blk na QEMU, NVMe para SSD e AHCI para SATA. A QEMU emula os
  três, e o mesmo driver roda em metal.
- **Camada de arquivos:** page cache sobre o VFS da fase 4.
- **Sistema de arquivos da raiz:** leitura e escrita.
- **Permissões em disco:** dono, grupo e modo, com ACL depois.
- **Leitura antecipada:** janela que cresce com a leitura sequencial, com um
  marcador na cauda assíncrona que dispara a próxima.
- **Descarte de página:** LRU de gerações, com refault como sinal. O MGLRU do
  Linux é a referência, não a cópia.
- **Entrada e som virtuais** (precisam de PCI): virtio-input para teclado, mouse e
  gamepad na QEMU, e virtio-snd para áudio. A especificação do virtio-snd (seção
  5.14 do virtio 1.3) ainda não foi lida: leia antes de escrever o driver.

Decisão aberta desta fase (caso 6 do `AGENTS.md`, porque o formato em disco é
contrato com o mundo de fora), a tomar antes de a fase 6 começar:

- **Formato da raiz:** compatível com ext4 (o Linux lê o disco, e há ferramentas
  para depurar) ou próprio (desenhado para a meta de memória e CPU).

### 7 — Usuários e sessões

- **Contas:** banco de contas e grupos, login em espaço de usuário, sessões.
- **Senha:** Argon2id (RFC 9106), implementado neste repositório. O kernel nunca vê
  senha.
- **Privilégio:** elevação por capacidade explícita e com prazo, sem `setuid root`
  amplo.
- **Auditoria:** log de login, elevação e negação de acesso.

### 8 — Instalação

Do jeito dos instaladores Linux: a pessoa sobe pela mídia live, escolhe instalar,
e o sistema vai para o disco.

1. **Mídia live:** `cargo xtask live-image` grava uma imagem GPT com partição EFI
   (Limine, kernel e o initramfs com o instalador), pronta para `dd` num pendrive.
   O gravador de GPT e FAT32 é do xtask, sem ferramenta externa. O Limine carrega
   tudo na RAM: não é preciso driver de pendrive para terminar a instalação.
2. **Menu ao entrar:** o menu do Limine oferece `RustOS (live)` e `Instalar
   RustOS` (linha de comando `installer`).
3. **Instalador:** em modo texto, no terminal da fase 0.5.
   - Escolher o disco: modelo, tamanho e partições que já existem.
   - Confirmar apagando por nome.
   - Criar a conta e a senha (Argon2id, fase 7).
   - Escolher o layout do teclado.
4. **Gravação:**
   - GPT com partição EFI e raiz;
   - formatação: FAT32 na partição EFI (escritor no sistema, além do do xtask) e
     a raiz no formato decidido na fase 6;
   - cópia de Limine, kernel e sistema;
   - entrada de boot no firmware pelos serviços de runtime do UEFI, para subir sem
     a mídia;
   - tudo com verificação de leitura depois de escrever.
5. **Entrada em PC real:** teclado e mouse USB por xHCI e HID, porque PS/2 quase
   não existe mais em máquina nova.
   O xHCI também faz DMA, sob o IOMMU da fase 6.
6. **Gamepad USB HID:** o mesmo driver HID entrega botões e eixos, no modelo de
   eventos do Linux (`BTN_GAMEPAD`, `ABS_*`). É o marco 3 dos jogos. A
   especificação de USB HID ainda não foi lida.

Decisão aberta desta fase (caso 6 do `AGENTS.md`):

- **Criptografia de disco:** opcional no instalador, sim ou não, e com que
  algoritmo. Fica numa camada de bloco abaixo do sistema de arquivos e não muda o
  formato decidido na fase 6.

### 9 — Gerência de recursos e energia

- **Grupos de recursos** no modelo do cgroup v2: peso e teto de CPU, teto e
  pressão de memória, máximo de processos, E/S.
- **Limites por processo** (rlimit) e política de falta de memória que escolhe a
  vítima por regra escrita.
- **Contabilidade por processo:** tempo de CPU e memória residente, e um `top`.
- **Energia:** idle states por CPU.
- **Pressão de memória (PSI) desde o início:** tempo parado por falta de memória,
  `some` e `full`, com gatilho por `poll()`. É barato, e é o sinal de que o
  daemon de pressão em espaço de usuário precisa.
- **`memory.high` antes de `memory.max`:** o primeiro desacelera o grupo e gera
  evento; o segundo é o limite duro. O processo é avisado antes de ser morto.
- **Compressão de memória com LZ4:** só em máquina sem swap em disco e com CPU
  ociosa. Compressão custa CPU: sem a CPU sobrando, não ganha.

### 10 — Rede

- **Placa e pilha:** virtio-net com interrupção que vira polling sob carga (modelo
  NAPI) e pilha TCP/IP própria.
- **Interfaces:** sockets da ABI Linux e cliente DNS.
- **Proteção:** filtro de pacotes básico, que nega por padrão o que entra.

### 11 — Interface gráfica

- **Compositor e entrada:** compositor sobre o framebuffer, janelas, mouse e
  teclado por evento.
- **Vídeo e aplicativos:** virtio-gpu 2D e aplicativos gráficos.
  - Page flip e vsync (marco 5 dos jogos). A especificação do virtio-gpu 2D ainda
    não foi lida.
  - Damage explícito por região, nunca a tela inteira.
  - Buffer do cliente em memória compartilhada com contagem de referência, sem
    cópia no caminho feliz.
- **SDL:** backend próprio de vídeo, eventos, áudio e timer (marco 4 dos jogos).
  SDL é código de terceiros: decisão futura, caso 2 do `AGENTS.md`.
- **Terminal:** o emulador da fase 0.5 vira um aplicativo.
- **Instalador:** ganha versão gráfica sobre o mesmo motor da fase 8.

### 12 — Endurecimento avançado

Cada item entra com o custo medido no SDD 07 e pode ser desligado só por regra
escrita.

- **Fluxo de controle:** CET (shadow stack e IBT) no x86_64; PAC, BTI e MTE no
  aarch64; CFI.
- **Políticas:** filtro de syscalls por processo (modelo seccomp/pledge) e controle
  de acesso obrigatório (MAC).
- **Boot verificado:** Secure Boot e measured boot com TPM 2.0, com a chave
  registrada pelo instalador. É decisão aberta (SDD 09).

### 13 — Jogos 3D

- **Pilha:** Mesa com virgl (GL) ou Venus (Vulkan) no guest, sobre a ABI Linux e o
  DRM, falando com a GPU do host pelo virtio-gpu.
- **Pré-requisitos:** fase 5 (a ABI Linux) e fase 11 (virtio-gpu e compositor).
- **Mesa é código de terceiros, grande:** decisão futura, caso 2 do `AGENTS.md`,
  e pesa contra a meta de memória.
- **Risco conhecido:** Venus no macOS/HVF não está no QEMU upstream (ver a escada,
  marco 6). Sem ABI Linux o 3D vira protocolo próprio.

## Jogos: a escada

Seis marcos, do mais barato ao mais caro. Cada um depende dos anteriores e entra na
fase em que sua dependência fica pronta. Fonte da escada: pesquisa de novembro de
2026; o que foi lido e o que não foi está na segunda tabela.

| Marco | O que é | Depende de | Fase |
| --- | --- | --- | --- |
| 1 | DOOM (ou Quake) por software sobre o framebuffer linear | Timer, entrada (PS/2 ou virtio-input), processo de usuário, ELF com libc | 5 (as fases 3 e 4 vêm antes) |
| 2 | Tempo e áudio: `clock_gettime(CLOCK_MONOTONIC)` e virtio-snd (ou Intel HDA) | PCI | 5 (tempo) e 6 (áudio) |
| 3 | Entrada de jogo: virtio-input na QEMU, gamepad USB HID em metal | xHCI | 6 (virtio-input) e 8 (HID) |
| 4 | SDL com backend próprio (vídeo, eventos, áudio, timer) | Marcos 1 a 3, compositor | 11 |
| 5 | Page flip e vsync em virtio-gpu 2D | Driver virtio-gpu | 11 |
| 6 | 3D acelerado: Mesa, virgl ou Venus | ABI Linux, DRM, virtio-gpu | 13 |

O marco 1 **não** precisa da fase 11: o jogo desenha direto no framebuffer.

| O que vale | Fonte | Marca |
| --- | --- | --- |
| QEMU: virtio-gpu com virgl (GL 4.3; 4.6 com blob), Venus e gfxstream | [QEMU virtio-gpu](https://www.qemu.org/docs/master/system/devices/virtio/virtio-gpu.html) | Primária, lida |
| Venus: Vulkan 1.1; o host precisa de `VK_KHR_external_memory_fd`; só Linux e Android | [Mesa Venus](https://docs.mesa3d.org/drivers/venus.html) | Primária, lida |
| Buffers compartilhados entre dispositivos: dma-buf | [kernel.org dma-buf](https://www.kernel.org/doc/html/latest/driver-api/dma-buf.html) | Primária, lida |
| Gamepad no Linux: `BTN_GAMEPAD` e `ABS_*` | [kernel.org gamepad](https://www.kernel.org/doc/html/latest/input/gamepad.html) | Primária |
| Venus no macOS/HVF não está no QEMU upstream: RFC de dez/2025, v3 de mar/2026 no patchew | patchew (lista de patches) | **Secundária**: não conferida no repositório do QEMU |
| O Redox faz isso com Orbital e QuakeSpasm | cookbook do Redox | **Secundária** |
| Proton e Wine pedem mais que SDL estático (`futex_waitv`, ABI Linux inteira) | pesquisa | **Sem fonte lida**: citado como risco pela pesquisa |
| virtio 1.3, seção 5.14 (som), virtio-gpu 2D (page flip, vsync), Intel HDA (CORB/RIRB), USB HID | [virtio 1.3](https://docs.oasis-open.org/virtio/virtio/v1.3/virtio-v1.3.html) | **Não lidos**: ler a fonte primária antes de cada driver |
| Lista de syscalls que um jogo usa | — | **Lacuna**: sai de rodar o marco 1 com `strace` equivalente |

## Memória ociosa: o alvo vem de uma medida

A meta "pouca memória" (SDD 01) não ganha número de chute. A pesquisa não achou
medida confiável de memória ociosa nem de Linux mínimo nem de Windows 11: o que
existe são requisitos de instalação (Windows 11 pede 4 GB, Haiku 256 MB para
subir) e números de blog, de 78 a 175 MB para distribuições pequenas em VM,
que servem só de ordem de grandeza. Por isso o alvo nasce de uma linha de base
medida por nós.

| Passo | O que se faz | Resultado |
| --- | --- | --- |
| 1. Linha de base | Linux mínimo (Alpine virt, ou `tinyconfig` com BusyBox e init) na **mesma** QEMU, mesma RAM e mesmo host que o RustOS. Espera N segundos sem carga e lê `MemAvailable` e `MemTotal`. Três boots frios, mediana | `X MB` ocupados de `N MB`, com versões anotadas |
| 2. RustOS | Mesma RAM, mesmo tempo de espera. O equivalente de `MemAvailable` é a memória livre do buddy mais o que o cache pode devolver. Kernel e espaço de usuário separados | `Y MB` ocupados de `N MB` |
| 3. Alvo | Fixado no [SDD 07](07_desempenho_e_recursos.md) como fração da linha de base, no formato "X MB de memória disponível ociosa com N MB de RAM, mediana de 3 boots" | Meta com número |

- **Quando:** o passo 1 não depende do RustOS e pode rodar já; o 2 e o 3 fecham a fase 5,
  quando há espaço de usuário para comparar.
- **Não usar `free` do BusyBox** como medida: a fórmula antiga subestima o ocupado.
- **Hoje** o RustOS só tem a medida do kernel: 249,3 MiB livres de 256 (x86_64) e
  247,0 MiB (aarch64) depois do boot (SDD 07). Não é comparável: sem espaço de
  usuário.

## Decisões futuras (casos do `AGENTS.md`)

Nada abaixo foi decidido. Cada item para o agente e abre PR em Draft **quando a
fase chegar**, antes de escrever código.

| Decisão | Caso | Fase | Por que é decisão |
| --- | --- | --- | --- |
| **SDL** como backend de jogos | 2 | 11 | Código de terceiros, grande: pesa na meta de memória. Alternativa: só API própria |
| **Mesa** (virgl e Venus) | 2 | 13 | Código de terceiros, muito maior que o SDL; exige DRM Linux no guest |
| **DRM do guest** para a Mesa | 6 | 13 | Contrato com o mundo de fora: sem ele o 3D vira protocolo próprio |
| **`AT_HWCAP`/`AT_HWCAP2`** e o equivalente de `ARCH_REQ_XCOMP_PERM` (AMX) e de `PR_SVE_SET_VL` | 6 | 5 | O programa lê o auxv e o sigframe depende do estado SIMD |
| **SIMD no kernel** (ChaCha20, `memset`) | 5; ou 3, se mudar o alvo de build | só se medido | Exige `asm!` fora de `arch/` ou alvo que não seja soft-float; hoje o código escalar basta |
| Formato da raiz; criptografia de disco | 6 | 6 e 8 | Já descritas nas fases 6 e 8 |
| Boot verificado | 2 e 4 | 12 | [SDD 09](09_seguranca.md) |
