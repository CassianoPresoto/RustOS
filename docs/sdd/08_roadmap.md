# SDD 08 — Roadmap

> **O que este capítulo responde:** em que ordem o sistema cresce, e o que marca o
> fim de cada fase.

Cada fase termina com `make pr` verde e um número novo no [SDD 07](07_desempenho_e_recursos.md).
A ordem segue dependência técnica: não há escalonador sem interrupção de timer,
nem espaço de usuário sem memória virtual.

| Fase | Entrega | Pronto quando |
| --- | --- | --- |
| **0 — Esqueleto** ✅ | Workspace hexagonal, boot pelo Limine em x86_64 e aarch64, alocador de frames, mapeamento de dispositivo (aarch64), serial, xtask, gates, CI | `make smoke ARCH=all` passa |
| **1 — Exceções e interrupções** | IDT/GDT/TSS (x86_64), `VBAR_EL1` (aarch64); Local APIC e timer TSC-deadline; GIC e generic timer; `PageMapper` do x86_64; lock que mascara interrupção (DEBT-0003, DEBT-0004); teste dentro do kernel | Page fault proposital vira mensagem na serial; tique do timer contado |
| **2 — Memória virtual e heap** | Tabelas próprias do kernel; buddy allocator com cache por CPU; `GlobalAlloc` para `alloc`; recuperação da memória do bootloader (DEBT-0006) | Medida de memória livre depois do boot no SDD 07 |
| **3 — Tarefas e SMP** | Troca de contexto em `arch/`, `core/sched` ligado ao kernel; outros núcleos pelo pedido MP do Limine; fila por CPU | Duas tarefas alternando em N núcleos; latência de troca de contexto medida |
| **4 — Espaço de usuário** | Anel 3 / EL0, entrada de syscall, carregador ELF, primeiro programa de usuário | Syscall nula medida |
| **5 — ABI Linux** | Adaptador de entrada que traduz syscalls Linux (modelo Asterinas/Kerla); `musl` estático rodando | Um binário Linux estático roda sem recompilar e é medido contra o Linux |
| **6 — Dispositivos** | virtio-blk, virtio-net, sistema de arquivos; descoberta por ACPI/DTB (DEBT-0002) | Benchmark de E/S ao lado do Linux |

Fases mais à frente (gráfico, jogos, rede de alto desempenho) dependem das medições
da fase 5 e entram aqui quando forem decididas.
