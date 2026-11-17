# SDD 08 — Roadmap

> **O que este capítulo responde:** em que ordem o sistema cresce, o que cada fase
> entrega (inclusive de segurança e de gerência de recursos) e o que marca o fim de
> cada uma.

Cada fase termina com `make pr` verde e um número novo no
[SDD 07](07_desempenho_e_recursos.md). A ordem segue dependência técnica: não há
escalonador sem interrupção de timer, nem espaço de usuário sem memória virtual,
nem usuário sem sistema de arquivos.

**Segurança não é uma fase.** Cada fase entrega os mecanismos que a tornam segura,
e o que é barato e difícil de pôr depois entra cedo (W^X, NX, SMEP/SMAP, PAN). O
modelo de ameaça e o porquê de cada mecanismo estão no [SDD 09](09_seguranca.md).

## A fila

| Fase | Entrega | Pronto quando |
| --- | --- | --- |
| **0 — Esqueleto** ✅ | Workspace hexagonal, boot pelo Limine em x86_64 e aarch64, alocador de frames, serial, xtask, gates, CI | `make smoke ARCH=all` passa |
| **0.5 — Tela e terminal** ✅ | Framebuffer, fonte própria, emulador VT/xterm, shell embutido, barra no topo | `fetch` desenhado na tela das duas ISAs (`make screenshot`) |
| **1 — Exceções, interrupções e tempo** | Vetores de exceção, controlador de interrupção, timer, relógio, entrada por interrupção, proteções de hardware ligadas | Page fault proposital vira relatório; tique contado; shell dorme em `hlt`/`wfi` |
| **2 — Memória virtual e heap** | Tabelas próprias com W^X, heap do kernel, aleatório do kernel, memória do bootloader recuperada | Escrita em `.text` falha; memória livre depois do boot medida |
| **3 — Tarefas, SMP e escalonador** | Troca de contexto, threads do kernel, fila por CPU, escalonador atrás de trait, idle sem tique | Latência de troca medida em N núcleos |
| **4 — Processos e isolamento** | Anel 3/EL0, syscall, carregador ELF, espaço de endereçamento por processo, handles com direitos | Syscall nula medida; processo que toca o kernel morre sem derrubar nada |
| **5 — ABI Linux e primeiros aplicativos** | Syscalls Linux traduzidas, credenciais Linux, musl estático, BusyBox | BusyBox roda sem recompilar e é medido contra o Linux |
| **6 — Arquivos e armazenamento** | VFS, page cache, initramfs, tmpfs, virtio-blk, NVMe, AHCI, sistema de arquivos com escrita, permissões, ACPI/DTB | Benchmark de E/S ao lado do Linux |
| **7 — Usuários e sessões** | Contas e grupos, login, Argon2id, elevação de privilégio por capacidade, auditoria | Dois usuários sem acesso aos arquivos um do outro |
| **8 — Instalação** | Imagem live de pendrive, teclado USB (xHCI), instalador em modo texto, GPT, partição EFI, entrada de boot UEFI | Instalar num disco vazio da QEMU e subir dele sem a mídia live |
| **9 — Gerência de recursos e energia** | Grupos de recursos (CPU, memória, processos, E/S), limites por processo, falta de memória, contabilidade, idle states | Grupo limitado respeita o limite sob carga |
| **10 — Rede** | virtio-net, pilha TCP/IP própria, sockets da ABI Linux, DNS, filtro de pacotes | Vazão e latência TCP ao lado do Linux |
| **11 — Interface gráfica** | Compositor, janelas, mouse, virtio-gpu 2D, aplicativos gráficos | Um aplicativo gráfico com tempo de quadro medido |
| **12 — Endurecimento avançado** | Mitigações de CPU por necessidade, CET, PAC/BTI, MTE, CFI, filtro de syscalls, MAC, boot verificado | Cada mecanismo com custo medido no SDD 07 |

Jogos com aceleração 3D dependem de driver de GPU e entram depois da fase 11,
quando houver medição para decidir.

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
- **Entrada por interrupção:** UART nas duas ISAs e teclado PS/2 no x86_64. Paga a
  DEBT-0007.
- **Base de memória e locks:** `PageMapper` do x86_64 (DEBT-0004) e lock que
  mascara interrupção (DEBT-0003).
- **Segurança:**
  - ligar WP, SMEP, SMAP e UMIP no x86_64 e PAN no aarch64;
  - relatar na serial o que a CPU tem e o que ficou ligado;
  - página de guarda em toda pilha de exceção.

### 2 — Memória virtual e heap

- **Tabelas próprias do kernel:**
  - `.text` só leitura e execução, `.rodata` só leitura, dados com NX;
  - página de guarda abaixo de toda pilha do kernel.
- **Heap:** buddy allocator com cache por CPU e `GlobalAlloc` para `alloc`.
- **Memória do bootloader recuperada** (DEBT-0006).
- **Aleatório do kernel:** semente de RDSEED/RNDR, gerador ChaCha20, e reposição da
  semente. É a base de KASLR, ASLR e canários.
- **KASLR:** exige kernel relocável. Entra com o custo de tamanho e de boot medido.
- **Zerar ao liberar:** política de zerar memória devolvida, com o custo medido.

### 3 — Tarefas, SMP e escalonador

- **Tarefas:** troca de contexto em `arch/`, threads do kernel, `core/sched` ligado
  ao kernel.
- **SMP:** outros núcleos pelo pedido MP do Limine e fila por CPU.
- **Escalonador:** política atrás de trait, do rodízio a uma política de prazo
  virtual no modelo EEVDF.
- **Tempo e energia:** sleep, timers de alta resolução e idle sem tique (NO_HZ_IDLE)
  para economizar energia.

### 4 — Processos e isolamento

- **Execução de usuário:** anel 3/EL0, entrada de syscall e carregador ELF.
- **Isolamento de memória:** espaço de endereçamento por processo, sem kernel
  gravável mapeado.
  - ASLR de usuário;
  - NX em pilha e heap;
  - cópia usuário↔kernel só por rotina que respeita SMAP/PAN.
- **Controle de acesso interno:** handles com direitos que só diminuem, no modelo
  Fuchsia/seL4. Nenhuma autoridade ambiente: um processo só acessa o que recebeu.
- **Ciclo de vida:** criar, terminar, esperar e sinais mínimos.

### 5 — ABI Linux e primeiros aplicativos

- **Adaptador de entrada:** traduz syscalls Linux para os serviços do núcleo, no
  modelo do Asterinas.
- **Credenciais Linux:** uid, gid, grupos e capabilities(7), traduzidos para os
  direitos internos. O modelo depende da decisão do SDD 09.
- **Primeiros aplicativos:** musl estático, BusyBox (`sh`, coreutils, `vi`, `top`),
  pipes, tabela de descritores e `/proc` mínimo.
- **Shell embutido:** vira console de recuperação.

### 6 — Arquivos e armazenamento

- **Camada de arquivos:** VFS, page cache, initramfs (módulos do Limine) e tmpfs.
- **Disco:** virtio-blk na QEMU, NVMe para SSD e AHCI para SATA; a QEMU emula os
  três, e o mesmo driver roda em metal.
- **Sistema de arquivos:** leitura e escrita; o formato da raiz é decisão aberta
  (abaixo).
- **Permissões:** dono, grupo e modo, com ACL depois.
- **Descoberta de hardware:** por ACPI e DTB, no lugar de endereço fixo (DEBT-0002).

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
   - GPT com partição EFI e raiz, formatação, cópia de Limine, kernel e sistema;
   - entrada de boot no firmware pelos serviços de runtime do UEFI, para subir sem
     a mídia;
   - tudo com verificação de leitura depois de escrever.
5. **Entrada em PC real:** teclado e mouse USB por xHCI e HID, porque PS/2 quase
   não existe mais em máquina nova.

Decisões abertas desta fase (caso 6 do `AGENTS.md`, porque o formato em disco é
contrato com o mundo de fora):

- **Sistema de arquivos da raiz:** compatível com ext4 (o Linux lê o disco, e há
  ferramentas para depurar) ou formato próprio (desenhado para a meta de memória
  e CPU).
- **Criptografia de disco:** opcional no instalador, sim ou não, e com que
  algoritmo.

### 9 — Gerência de recursos e energia

- **Grupos de recursos** no modelo do cgroup v2: peso e teto de CPU, teto e
  pressão de memória, máximo de processos, E/S.
- **Limites por processo** (rlimit) e política de falta de memória que escolhe a
  vítima por regra escrita.
- **Contabilidade por processo:** tempo de CPU e memória residente, e um `top`.
- **Energia:** idle states por CPU.

### 10 — Rede

- **Placa e pilha:** virtio-net com interrupção que vira polling sob carga (modelo
  NAPI) e pilha TCP/IP própria.
- **Interfaces:** sockets da ABI Linux e cliente DNS.
- **Proteção:** filtro de pacotes básico, que nega por padrão o que entra.

### 11 — Interface gráfica

- **Compositor e entrada:** compositor sobre o framebuffer, janelas, mouse e
  teclado por evento.
- **Vídeo e aplicativos:** virtio-gpu 2D e aplicativos gráficos.
- **Terminal:** o emulador da fase 0.5 vira um aplicativo.
- **Instalador:** ganha versão gráfica sobre o mesmo motor da fase 8.

### 12 — Endurecimento avançado

Cada item entra com o custo medido no SDD 07 e pode ser desligado só por regra
escrita.

- **Mitigações de CPU:** só onde a CPU precisa (KPTI, Spectre v2, MDS), e relatadas
  no boot.
- **Fluxo de controle:** CET (shadow stack e IBT) no x86_64; PAC, BTI e MTE no
  aarch64; CFI.
- **Políticas:** filtro de syscalls por processo (modelo seccomp/pledge) e controle
  de acesso obrigatório (MAC).
- **Boot verificado:** Secure Boot e measured boot com TPM 2.0, com a chave
  registrada pelo instalador. É decisão de plataforma (SDD 09).
