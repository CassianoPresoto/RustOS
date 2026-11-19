# SDD 10 — Exceções, interrupções e tempo

> **O que este capítulo responde:** como o kernel recebe exceção e IRQ em cada
> ISA, de onde vem o tique e o relógio, e que proteções de hardware ligam no boot.

O núcleo vê as duas ISAs pelas mesmas portas (`hal::Handlers`, com `fault`, `irq` e
`preempt`; `hal::Fault`; `hal::Cpu`) e pelo contrato de `kernel/src/platform/mod.rs`. Tudo o que muda
entre elas está numa seção por ISA, abaixo.

## x86_64

### Sequência de `platform::init_interrupts`

| Passo | O quê | Onde |
| --- | --- | --- |
| 1 | WP (`CR0`), SMEP, SMAP, UMIP (`CR4`) e NXE (`EFER`), só o que o CPUID anuncia; o estado lido de volta vira `protections()` | `arch/x86_64/src/protect.rs` |
| 2 | `platform::active_space`: lê o `CR3` e caminha as tabelas pelo mapa direto | `mmu.rs`, `pte.rs`, `core/mm/src/tables.rs` |
| 3 | Três pilhas de 4 páginas com página de guarda (`Memory::map_stack`), para #DF, NMI e #MC | `kernel/src/memory.rs` |
| 4 | GDT, TSS e IDT próprias; `lgdt`, CS por retorno distante, `ltr`, `lidt` | `traps.rs`, `segments.rs` |
| 5 | 8259 mascarado (0xFF em 0x21 e 0xA1) | `pic.rs` |
| 6 | Local APIC: x2APIC por MSR se `CPUID.1:ECX[21]`, senão xAPIC por MMIO na janela de dispositivos | `apic.rs`, `lapic.rs` |
| 7 | I/O APIC em `0xFEC0_0000` (DEBT-0009): pino 4 (COM1) → vetor `0x24`; pino 1 (teclado) → `0x21`, mascarado | `ioapic.rs` |
| 8 | Calibração: uma janela do PIT (canal 2) de 5 a 20 ms, lida pelo contador, fecha quando a incerteza cai a 400 ppm ou não vai cair ([abaixo](#calibração-pelo-pit-uma-janela-que-cresce-até-fechar)); mede o TSC e o contador do timer do APIC; o timer fica em modo de um tiro (TSC-deadline se o CPUID anuncia), desarmado: `interrupts::init` arma o primeiro tique | `pit.rs`, `clock.rs`, `lapic.rs` |
| 9 | `Uart16550::enable_receive_interrupt` (`IER` bit 0 e `OUT2` do `MCR`), por último | `drivers/uart16550` |

O chamador (`interrupts::init`) liga o `IF` depois. Os núcleos secundários repetem só o que é
por CPU ([abaixo](#núcleos-secundários-x86_64)). A parte pura de cada passo
roda no `cargo test` do host, com `RegisterBus` e memória de tabelas falsos.

### Vetores

| Vetor | Quem | Pilha |
| --- | --- | --- |
| 0–31 | Exceções da CPU; tem código de erro 8, 10–14, 17, 21, 29 e 30 (`fault::ERROR_CODE_VECTORS`) | atual |
| 8 (#DF), 2 (NMI), 18 (#MC), 1 (#DB) | IST1, IST2, IST3 e IST4 da TSS (o campo do gate é o índice mais um). Os quatro entram pela **entrada paranoica**, que decide o `GS` pelo sinal de `IA32_GS_BASE`; o `#DB` vindo do anel 3 copia o quadro para a pilha da tarefa e segue como entrada normal | própria |
| `0x20` | Timer do Local APIC | atual (a da tarefa interrompida) |
| `0x21` | Teclado PS/2 (pino 1, mascarado) | atual |
| `0x24` | COM1 (pino 4) | atual |
| `0xFC` | IPI de invalidação do TLB (`IrqSource::Shootdown`): a CPU lê a caixa dela em `kernel/src/tlb.rs`, invalida a faixa, baixa o bit e dá o EOI; o iniciador espera todas ([SDD 17](17_processos_isolamento_e_syscalls.md#a-fatia-4c-paginação-sob-demanda-e-shootdown-como-entrou)) | atual |
| `0xFD` | IPI de reescalonar (`IrqSource::Reschedule`) | atual |
| `0xFE` | IPI de parada, pânico em outro núcleo (`IrqSource::Stop`) | atual |
| `0xFF` | Espúrio: chega ao `Handlers.irq` e não recebe EOI | atual |

Os 256 gates apontam para stubs de 16 bytes (`rustos_isr_stubs + 16 * vetor`) que
empilham um código de erro falso onde a CPU não empilha e o número do vetor. A
entrada comum salva os 15 registradores (ordem de `TrapFrame`), faz `cld` e chama
`trap_dispatch`, que decodifica em `hal::Fault` (page fault pelos bits do código de
erro e pelo `CR2`) ou entrega `hal::Irq` ao núcleo, dá o EOI depois do handler e chama
`Handlers.preempt`, onde o núcleo pode trocar de tarefa ([SDD 15](15_tarefas_e_preempcao.md)).
O breakpoint volta; qualquer outra exceção pára a CPU, **menos a que vem do anel 3**: com o `CS` salvo em RPL 3 a entrada faz `swapgs` (e outro na saída), `Fault.from_user` fica ligado e o kernel mata só o processo, **ou, se a falta de página cabe numa região preguiçosa, a resolve e o `iretq` repete a instrução** (4c). A falta de página no `rip` da cópia de usuário pergunta ao núcleo (`Handlers.copy_fault`): resolvida, a cópia se repete; senão volta ao ponto de recuperação. `NMI`, `#DF` e `#MC` seguem fatais mesmo de usuário. A entrada por `SYSCALL` (`rustos_syscall_entry`) monta o mesmo quadro, com o vetor `0x100`, e sai por `SYSRET` (se a conferência passa) ou pelo `IRETQ`; a volta ao usuário limpa a pilha do kernel e exige `RIP` e `RSP` canônicos ([SDD 17](17_processos_isolamento_e_syscalls.md#a-fatia-4a-como-entrou)).

### Núcleos secundários (x86_64)

O Limine entrega cada núcleo com o estado do BSP (GDT, tabelas e registradores de controle do
Limine) e o `goto_address` ([SDD 04](04_boot.md)). O que o kernel refaz **em cada um**, na ordem:

| Passo | O quê | Onde |
| --- | --- | --- |
| 1 | `IA32_GS_BASE` aponta o bloco `PerCpu` (`KERNEL_GS_BASE` fica em zero, o `GS` do usuário, que o `swapgs` troca a cada transição de anel: [SDD 17](17_processos_isolamento_e_syscalls.md#x86_64-syscallsysret)); `gs:[0]` devolve o endereço dele, e os campos seguintes (`user_rsp`, `kernel_rsp`, o `rsp[0]` da TSS, a raiz ativa) são o `CpuPrefix` que o `asm` de entrada lê | `percpu.rs` |
| 2 | WP, SMEP, SMAP, UMIP e NXE, e só então o `CR3` do kernel (o NXE tem de valer antes das tabelas com NX) | `protect.rs`, `mmu.rs` |
| 3 | Troca para a pilha própria (`call_on_stack`) | `stack.rs` |
| 4 | **GDT e TSS próprias** (`CpuTables`) com as 4 pilhas IST próprias, e `lidt` da **IDT compartilhada** (montada uma vez pela CPU de boot). O `ltr` marca a TSS como ocupada na GDT, por isso uma GDT por CPU. Carregar o seletor nulo no `GS` pode zerar a base: `load_cpu` lê a `GS_BASE` antes e a escreve depois | `traps.rs` |
| 5 | Local APIC no modo que a CPU de boot escolheu (x2APIC por MSR, ou xAPIC na mesma página MMIO, que cada núcleo vê como a sua), vetor espúrio, timer mascarado e depois no mesmo modo de um tiro | `apic.rs` |
| 6 | `CR0`, `CR4` e `XCR0` de FPU/SIMD | `fpu.rs` |

O contador do timer e o TSC são calibrados uma vez, no BSP, e valem para todos: **supõe TSC
sincronizado e a mesma frequência do contador em cada núcleo** (não verificado em metal,
[DEBT-0057](../tech-debt/0057-tsc-entre-nucleos-sem-verificacao.md)).

**IPI** (`lapic.rs`, `apic.rs`): entrega fixa, destino físico, nível *assert*, borda. No x2APIC o ICR é o MSR `0x830`, de 64 bits, com o LAPIC ID em 63:32; no xAPIC são dois registradores, `ICR2` (`0x310`, destino em 31:24) escrito antes do `ICR` (`0x300`), depois de esperar o bit de envio pendente (bit 12). Antes do ICR vai um `mfence` (`weak_wrmsr_fence` do Linux, `x2apic_phys.c`): o `wrmsr` do x2APIC não ordena as escritas anteriores, e o IPI de reescalonar precisa que `kick_pending` e a fila estejam visíveis ao destino. "Todos menos eu" é o atalho `0b11` nos bits 19:18, sem destino.

### Seletores

| Seletor | Descritor |
| --- | --- |
| `0x08` | Código do kernel (64 bits) |
| `0x10` | Dados do kernel |
| `0x18` (RPL 3) | Dados do usuário |
| `0x20` (RPL 3) | Código do usuário (dados antes do código, a ordem do `sysret`) |
| `0x28` | TSS de 16 bytes (ocupa duas entradas) |

### Tempo

O timer do Local APIC é de um tiro: no modo TSC-deadline se `CPUID.1:ECX[24]`
(`IA32_TSC_DEADLINE`), senão pelo contador, divisor 16, com a contagem `apic_hz × (prazo − agora)`.
Até `task::init` o núcleo o rearma a cada disparo para o próximo tique de `TIMER_HZ`
(o autoteste `timer:`); depois, só para o menor entre o fim da fatia da corrente e o
próximo acordar, e sem tique com tudo ocioso, salvo a saturação do contador de 32 bits do APIC, que acorda a ociosa a cada ~69 s ([SDD 15](15_tarefas_e_preempcao.md)). O relógio (`now_ns`) é o TSC menos a leitura da calibração,
convertido por `ticks * mult >> shift` (`hal::TickScale`, o `cyc2ns` do Linux; sem divisão de 128 bits nem ponto flutuante). Sob a QEMU
`-cpu max` em TCG não há TSC-deadline nem TSC invariante, e o vendor é AMD, o que
inutiliza o CPUID 0x15 do Linux: por isso a referência é o PIT.

| Medida (QEMU 11.1.3, TCG, `make smoke`) | Valor |
| --- | --- |
| TSC | ~1,0000 GHz (o TCG conta nanossegundos) |
| Contador do timer do APIC, divisor 16 | ~62,5 MHz |
| `timer: 100 Hz, 5 ticks in …` | 40 a 45 ms (a fase do primeiro tique é aleatória) |
| `sched: … timer one-shot` | o contador do APIC (62,5 MHz); o TSC-deadline nunca rodou ([DEBT-0034](../tech-debt/0034-tsc-deadline-sem-teste-no-x86.md)) |

### Calibração pelo PIT: uma janela que cresce até fechar

A calibração era de 20 ms de PIT, a maior fatia do `interrupts::init` (em TCG o PIT corre em tempo real). O TSC da QEMU em TCG é de 1 GHz exatos (o TCG conta nanossegundos), o que dá o erro de cada medida. Janela fixa lida pela saída do canal 2, quatro boots por janela:

| Janela | Erro máximo visto |
| --- | --- |
| 20 ms | 0,07% |
| 5 ms | 0,28%, sempre para mais |
| 2,5 ms | 0,84% |

O erro vem da latência da leitura de porta que fecha a janela (cerca de 7 a 14 µs no TCG), não da quantização. A versão atual (`arch/x86_64/src/pit.rs`) segue o `quick_pit_calibrate` do Linux (`arch/x86/kernel/tsc.c`, v6.18, linhas 568–641, lido no GitHub): o PIT conta em modo 0 a partir de 0xFFFF, e o TSC é amostrado junto do contador (e do contador do APIC), numa **única** janela; fecha quando a incerteza, `d_origem + d_agora` sobre o intervalo, cai abaixo do alvo, com o teto de duração (`MAX_QUICK_PIT_MS`, 50 ms lá) e a saída antecipada quando a extrapolação mostra que o alvo não chega (`i == 1 && d1 + d2 >= (delta * MAX_QUICK_PIT_ITERATIONS) >> 11`, lá).

| | Linux (`quick_pit_calibrate`) | RustOS (`pit::calibrate`) |
| --- | --- | --- |
| Sinal do PIT | o byte alto do contador muda a cada 256 ticks (`pit_expect_msb`, lendo `0x42` duas vezes, sem latch) e o TSC se toma na última leitura com o mesmo valor | o comando de latch (`0x80`) trava a contagem de 16 bits e o instante é o do comando, entre duas leituras do TSC: não precisa alinhar a transições |
| Incerteza | `d1 + d2` (período de leitura das duas pontas) contra `delta >> 11` (~488 ppm) | a largura das duas amostras das pontas, `(d_origem + d_agora)/2`, sobre o intervalo, mais 1 tick do PIT (`1e6 / ticks` ppm: a diferença entre dois latches erra até 838 ns) |
| Alvo | 500 ppm | 400 ppm (margem) |
| Teto | 50 ms | 20 ms |
| Se o alvo não chega | falha e o Linux cai para outro método (`pit_hpet_ptimer_calibrate_cpu`, `CAL_MS` 10, `CAL2_MS` 50) | aceita o que a janela mínima de 5 ms deu, se a incerteza for de até 1% (10 000 ppm); acima disso recusa (`MissingFeature("pit: …")`) |
| PIT parado | sai por contagem de leituras (`count < 50000`) | sai por ciclos de TSC (300 M, ~100 a 300 ms) |
| Primeira leitura | descartada (`pit_verify_msb(0)`) | descartada; a origem tem de estar a até 4 096 ticks de 0xFFFF, senão a contagem é velha e a medida é recusada |

No TCG a leitura de porta é lenta (cerca de 7 µs por operação): a incerteza a 5 ms é de uns 1 700 a 2 700 ppm e a 20 ms seria de uns 500, então a saída antecipada fecha aos 5 ms, sempre, sem medir duas vezes. Uma primeira versão deste PR media 5 ms, e se a incerteza passasse de 500 ppm media de novo com uma janela maior: o boot normal do x86_64 oscilava entre 24 e 40 ms. Com uma janela só, 12 boots (TSC medido contra 1 GHz): **erro máximo 0,094%, mediana 0,037%** (a incerteza devolvida, 1 650 a 2 660 ppm, é um limite, não o erro); kernel do boot normal **26 a 31 ms, mediana 26**, sem as duas modas. O limite é de 3 a 5 vezes o erro visto, e o boot só imprime `tsc: calibrated to within N ppm` acima de 5 000 ppm (0,5%); o modo de teste mostra o limite sempre, na linha `boot phases:` (`tsc error at most N ppm`). Em hardware de leitura rápida (1 µs) a medida fecha em 400 ppm antes dos 5 ms (teste de host com 0,5 µs por leitura). O aarch64 não calibra (o `CNTFRQ_EL0` dá a frequência). Em metal ou KVM ninguém mediu ([DEBT-0066](../tech-debt/0066-calibracao-do-pit-com-janela-curta-sem-medida-em-kvm.md)).

Com `-machine q35,pit=off` o contador não anda: `interrupts: MissingFeature("pit: the counter does not move")` depois de ~0,3 s, e o boot para (antes, com 50 M consultas, a calibração lixo era aceita: TSC 480× errado, `kernel 11624 ms`).

### Custo

O tamanho do kernel antes e depois da fase 1 está no SDD 07 (x86_64 `release`:
100 541 → 132 200 bytes carregados, já com o teclado). Os 256 stubs são 4 096
bytes de `text` e a IDT, 4 096 de `bss`; o resto é o despacho, o APIC, a
calibração e o teclado.

### Memória virtual acrescentada

| Faixa | O quê |
| --- | --- |
| `0xffff_fe80_0000_0000`, 1 GiB | Janela de pilhas (entrada 509), com página de guarda abaixo de cada pilha |
| `0xffff_ff00_0000_0000`, 1 GiB | Janela de dispositivos (entrada 510): Local APIC (xAPIC) e I/O APIC, com `PCD\|PWT` (PAT[3] = UC no layout do Limine) |

Desde a fase 4b há uma terceira janela, de 1 GiB, a de módulos (initramfs só leitura), logo depois da de pilhas na entrada que o `free_top_level_slot` escolhe ([SDD 13](13_memoria_virtual.md#o-módulo-initramfs)).

### Fontes

| Afirmação | Fonte |
| --- | --- |
| Vetores com código de erro | `exception_has_error_code` em `target/i386/tcg/seg_helper.c` da QEMU (8, 10–14, 17) e `arch/x86/kernel/traps.c`, `idtentry.h` do Linux (também 21, 29); a tabela 6-1 da SDM vol. 3A não foi lida direto |
| Bits do código de erro do #PF | `arch/x86/include/asm/trap_pf.h` do Linux |
| x2APIC: `CPUID.1:ECX[21]`, `IA32_APIC_BASE` EN (bit 11) e EXTD (bit 10) | `CPUID_EXT_X2APIC` e `MSR_IA32_APICBASE_*` em `target/i386/cpu.h` da QEMU; `X2APIC_ENABLE` em `apicdef.h` do Linux |
| Registradores do APIC e MSR x2APIC `0x800 + (deslocamento >> 4)` | `arch/x86/include/asm/apicdef.h` do Linux |
| Calibração pelo PIT (porta 0x61, canal 2, `0xB0`) | `pit_calibrate_tsc` em `arch/x86/kernel/tsc.c` do Linux; porta 0x61 em `hw/audio/pcspk.c` da QEMU (bit 5 é o OUT2) |
| Frequência do PIT 1 193 182 Hz | `PIT_FREQ` em `include/hw/timer/i8254.h` da QEMU |
| I/O APIC em `0xFEC0_0000` | `IO_APIC_DEFAULT_ADDRESS` em `include/hw/intc/ioapic.h` da QEMU |
| PAT do Limine (PAT3 = UC) | `PROTOCOL.md` do Limine, seção *Caching* |
| IST no gate é índice mais um | comentário em `arch/x86/kernel/idt.c` do Linux |
| `IA32_GS_BASE` = `0xC000_0101`, `IA32_KERNEL_GS_BASE` = `0xC000_0102` | `msr-index.h` do Linux (master, lido em 2026-11-17) |
| ICR em `0x300`/`0x310`, `APIC_DEST_ALLBUT` = `0xC0000`, `APIC_INT_ASSERT` = `0x04000`, `APIC_ICR_BUSY` = `0x01000`, `APIC_DM_FIXED` = 0 | `apicdef.h` do Linux (lido em 2026-11-17) |
| Escrita do ICR no x2APIC: um `wrmsr` de `id << 32 \| low` no MSR `0x800 + (0x300 >> 4)` | `native_x2apic_icr_write` em `arch/x86/include/asm/apic.h` do Linux |
| Que o Limine entrega o AP com o estado do BSP, e o `goto_address` com store de release | `PROTOCOL.md`, seção *MP*, do Limine |
| Ordem de init de um AP (`cpu_init`, `ap_starting`, `set_cpu_online`) | `arch/x86/kernel/smpboot.c` do Linux, `start_secondary` (lido para a ordem; o Linux sobe o AP por INIT/SIPI, o que aqui é do Limine) |
| Bits do ICR (modo, nível, atalho) na SDM vol. 3A, seção 11.6.1 | **não lida direto**: conferido só pelo `apicdef.h` do Linux acima |

## aarch64

Fonte dos números: ARM ARM via `esr.h` do Linux (classes e `FSC`), `entry.S` do
Linux (ordem dos vetores), `arm-gic-v3.h` do Linux e `hw/intc/arm_gicv3_*.c` da
QEMU (registradores), `hw/arm/virt.c` (INTIDs e endereços). Cada decisão abaixo
foi conferida nelas.

| Peça | Como é |
| --- | --- |
| Vetores | `VBAR_EL1` aponta para `rustos_vectors` (`arch/aarch64/src/exception.rs`): 2 KiB, 16 entradas de 0x80. O kernel roda em EL1h (`call_on_stack` seleciona `SP_EL1`), então é interrompido no grupo "EL1 com SPx" (4–7); o grupo SP0 usa o mesmo corpo |
| Pilha | IRQ empilha na pilha da própria tarefa (`SP_EL1`), e a exceção vinda de EL0 (vetores 8 a 11) também: o quadro de 288 bytes guarda o `SP_EL0` e o `TPIDR_EL0` do usuário, e a volta a EL0 limpa a pilha, restaura os dois e zera o `TPIDRRO_EL0`. Síncrona e SError de EL1h (4 e 7) pegam a pilha de falhas **desta CPU**, de 4 páginas com guarda, cujo topo `install` e `install_secondary` gravam no bloco `PerCpu` (o `vector_fault` o lê pelo `TPIDR_EL1`, guardando o `x0` no `TPIDRRO_EL0`; sem o topo ainda, no começo do boot, usa o `SP_EL0` da pilha precoce) ([SDD 15](15_tarefas_e_preempcao.md)). **Desenho que a 4a não fez:** a ordem do Linux, toda síncrona na pilha da tarefa e só o estouro na de falhas ([SDD 17](17_processos_isolamento_e_syscalls.md#aarch64-svc), [DEBT-0078](../tech-debt/0078-aarch64-excecao-do-el1h-na-pilha-de-falhas.md)) |
| Quadro | x0–x30, `ELR_EL1`, `SPSR_EL1`: 272 bytes. `brk` volta para a instrução seguinte (o `ELR` aponta para o próprio `brk`: soma 4); falha não recuperável para a CPU em `wfe` |
| `ESR_EL1` | `esr.rs` (puro): classe → `hal::FaultKind`; em aborto, `DFSC`/`IFSC` (tradução por nível, acesso, permissão, alinhamento) e `WnR`. `Fault.code` é o `ESR` cru |
| GICv3 | `drivers/gicv3` (MMIO) mais `gic_cpu.rs` (`ICC_*_EL1`). Distribuidor em `0x0800_0000` e a região dos redistribuidores em `0x080A_0000` (`0x20000` por CPU), pela janela de dispositivos (DEBT-0002). Sequência: distribuidor desligado → `ARE` + grupo 1 → **acha o redistribuidor desta CPU** → acorda-o (`GICR_WAKER`) → `ICC_SRE_EL1`, `PMR`, `IGRPEN1` |
| Redistribuidor por CPU | `find_redistributor` percorre os frames da região a partir do primeiro, lê `GICR_TYPER` e compara a afinidade (`[63:32]`) com a do `MPIDR_EL1` da CPU; para em `Last` (bit 4) sem achar. Cada frame tem 128 KiB (`RD_base` e `SGI_base`), ou 256 KiB com `VLPIS` (bit 1; GICv4: mais `VLPI_base` e uma página reservada). Cada CPU só mexe no frame dela |
| IPI | `dsb ishst` e depois `ICC_SGI1R_EL1` (a barreira de `gic_ipi_send_mask` do Linux: as escritas em memória normal ficam visíveis às outras CPUs antes da SGI), SGI do grupo 1: INTID 0 reescalonar, 1 parar; `Aff3.Aff2.Aff1` no registrador, `TargetList` = `1 << (Aff0 % 16)` e `RS` = `Aff0 / 16`; "todos menos eu" é `IRM = 1`. As duas SGIs são habilitadas, prioridade `0x40`, em **cada** redistribuidor (`ISENABLER0`). **Não há SGI de invalidação do TLB:** o `tlbi ...is` do `MmuControl::entry_removed` alcança todas as CPUs e o `dsb ish` espera ([SDD 17](17_processos_isolamento_e_syscalls.md#a-fatia-4c-paginação-sob-demanda-e-shootdown-como-entrou)) |
| Fontes | PPI 27 (`CNTV`, nível) no redistribuidor **de cada CPU** e SPI 33 (UART0, nível) no distribuidor, grupo 1, roteada para a afinidade da CPU de boot ([DEBT-0056](../tech-debt/0056-irqs-de-dispositivo-so-na-cpu-de-boot.md)) |
| Núcleo secundário | Em EL1t pelo Limine; `call_on_stack` passa a EL1h; depois, em ordem: `MAIR` (índice 7) e `PAN` na entrada, `TTBR1_EL1` do kernel (`Aarch64Mmu::activate`); no `ap_main`, `VBAR_EL1` e a pilha de falhas no bloco `PerCpu` (`install_secondary`), o redistribuidor, `ICC_*`, o PPI do timer e as SGIs, `CNTV` ligado e desarmado, `CPACR_EL1.FPEN`. `MAIR_EL1`, `SCTLR_EL1`, `TCR_EL1` e `CPACR_EL1` são por CPU |
| EOI | `EOImode` 0: um `ICC_EOIR1_EL1` baixa a prioridade e desativa, antes de `Handlers.preempt`. INTID ≥ 1020 (espúrio) não leva EOI |
| Timer | `CNTV` (o Linux usa o virtual em EL1 sem EL2 e o HVF só entrega o virtual). `CNTFRQ_EL0` lido em tempo de execução. O disparo usa `CNTV_CVAL_EL0` absoluto (`TVAL` acumulava ~2 ms de atraso por tique sob QEMU: 50 tiques em 600 ms), de um tiro: o núcleo o rearma a cada disparo para o próximo tique ou acordar |
| Relógio | `now_ns` = `(CNTVCT − início) × mult >> shift` (`hal::TickScale` de `CNTFRQ`, sem divisão; um contador atrás da origem lê zero); zero em `init_interrupts`. `sched_clock_ns` é o mesmo sem o `isb` antes do `mrs`, só para o escalonador ([SDD 15](15_tarefas_e_preempcao.md), "Tempo") |
| PL011 | `IMSC.RXIM \| RTIM`; `ICR` limpo no começo da IRQ, antes de drenar o FIFO: o byte que chega depois rearma a IRQ (limpar depois perdia o acordar no modelo da QEMU) |
| PAN | ligado se `ID_AA64MMFR1_EL1.PAN` ≠ 0, com `SCTLR_EL1.SPAN` = 0 (liga na entrada da exceção) e `PSTATE.PAN` = 1 agora; `protections:` mostra o `PSTATE.PAN` lido de volta |

A mesma configuração roda no HVF (vGIC da Apple, `virt-11.1`+, macOS 15+) e no TCG:
a `virt` fixa o GICv3 por `gic-version=3` e as duas aceleram os mesmos registradores.

### Fontes (aarch64, núcleos)

| Afirmação | Fonte |
| --- | --- |
| Campos de `ICC_SGI1R_EL1` | `ICC_SGI1R_*` em `include/linux/irqchip/arm-gic-v3.h` do Linux (lido em 2026-11-17) |
| `GICR_TYPER.VLPIS` (bit 1) e `Last` (bit 4); `Affinity_Value` em `[63:32]` | `GICR_TYPER_*` no mesmo arquivo |
| Percorrer os redistribuidores: 128 KiB por frame, mais 128 KiB com `VLPIS`, até `Last` | `gic_iterate_rdists` em `drivers/irqchip/irq-gic-v3.c` do Linux (lido em 2026-11-17); o Linux também aceita `redistributor-stride` do DT, que aqui não há (DEBT-0002) |
| O Limine entrega `X0` = `MpInfo` com `mpidr` mascarado e `extra_argument` livre | `PROTOCOL.md`, *MP*, do Limine |
| Ler o `GICR_TYPER` de cada frame com acesso de 64 bits | `read64` do `RegisterBus` (dois de 32 bits na ordem baixo, alto): a QEMU aceita; **o Arm IHI 0069 não foi lido** neste ponto |
| O PSCI (`CPU_ON`) não é usado: quem acorda os núcleos é o Limine | `PROTOCOL.md`; a especificação (DEN0022) **não foi lida** |
