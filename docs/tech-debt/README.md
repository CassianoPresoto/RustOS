# Dívidas técnicas conscientes

Uma dívida por arquivo, numerada: `0001-titulo-curto.md`, a partir de
[`_template.md`](_template.md).

**Dívida consciente é decisão; dívida esquecida é defeito.** A diferença inteira
está no gatilho — o *evento* que obriga a pagar. Por isso ele é campo de
frontmatter e `make docs-check` recusa dívida aberta sem ele.

| Status | Significa |
| --- | --- |
| `Open` | Aceita, não paga. Vale o gatilho. |
| `Paid` | Resolvida. Sai da pasta no PR que a paga, junto com as referências no código. |
| `Dropped` | Deixou de fazer sentido. Diga por quê antes de apagar. |

**Próximo número:** o maior número entre os arquivos desta pasta, mais um. Número
não se reaproveita.

Dívida que já tem dono e prazo é issue, não item aqui.

## Índice

| # | Dívida | Status | Gatilho |
| --- | --- | --- | --- |
| [0001](0001-boot-pelo-limine.md) | Boot depende do Limine, binário de terceiros | Open | O tempo do bootloader passar a ser a maior parcela do boot medido em metal, ou o primeiro alvo sem UEFI |
| [0002](0002-dispositivos-no-endereco-da-qemu-virt.md) | Endereços da PL011 e do GICv3 fixos nos da QEMU `virt` | Open | A primeira placa aarch64 que não seja a QEMU `virt` |
| [0008](0008-terminal-sem-caractere-largo.md) | Terminal sem caractere largo nem combinante | Open | O primeiro programa da fase 5 (vi, less) mostrando CJK, emoji ou acento decomposto no terminal, ou a fonte ganhar glifo de largura dupla |
| [0009](0009-ioapic-no-endereco-fixo-da-qemu.md) | I/O APIC no endereço fixo e IRQs ISA sem redirecionamento do MADT | Open | A primeira máquina x86_64 que não seja a QEMU q35, ou o parser de ACPI da fase 6 |
| [0010](0010-teclado-sem-comandos-ao-dispositivo.md) | Teclado sem comandos ao dispositivo, LEDs nem Num Lock | Open | O primeiro boot em metal com um teclado PS/2 real |
| [0014](0014-x86-sem-salvaguardas-de-metal.md) | x86_64 sem as salvaguardas que só o metal pede | Open | O primeiro boot do x86_64 em metal |
| [0015](0015-cripto-sem-medida-no-x86-acelerado.md) | Criptografia do núcleo sem medida do x86_64 com aceleração | Open | O primeiro boot do x86_64 com KVM ou em metal |
| [0017](0017-dupla-liberacao-escondida-no-cache-de-frames.md) | Dupla liberação de frame no cache por CPU só aparece ao esvaziar o cache | Open | O primeiro defeito de dupla liberação de frame que o cache esconder, ou o modo de depuração do kernel |
| [0019](0019-aleatorio-sem-fonte-de-hardware-fraco.md) | Aleatório só com jitter do contador em CPU sem gerador de hardware | Open | O primeiro consumidor de segurança do aleatório (KASLR, canário, ASLR de usuário) rodando em CPU sem RDSEED, RDRAND nem RNDR |
| [0021](0021-mapa-direto-sem-acpi-nem-reservada-mapeada.md) | Mapa direto sem as tabelas ACPI nem as regiões reservadas que o Limine mapeia | Open | O primeiro código que lê ACPI (MADT, MCFG) ou outra região reservada mapeada, na fase 6 |
| [0024](0024-aarch64-pie-com-textrel.md) | Kernel aarch64 relocável só com `-z notext` (TEXTREL) | Open | A `core` pré-compilada do alvo aarch64 passar a vir com relocação PIC, ou o primeiro build com build-std |
| [0026](0026-troca-de-tabela-do-aarch64-sem-conferencia.md) | Troca de tabela do aarch64 sem a leitura do Arm ARM nem a conferência do MAIR | Open | O primeiro boot em metal aarch64, ou a leitura do Arm ARM nas seções de TLB e de MAIR |
| [0027](0027-virtio-snd-sem-a-leitura-da-spec.md) | Driver de virtio-snd sem a leitura da especificação | Open | Antes de escrever o driver de virtio-snd (fase 6) |
| [0028](0028-virtio-gpu-2d-sem-a-leitura-da-spec.md) | Driver de virtio-gpu 2D sem a leitura da especificação | Open | Antes de escrever o driver de virtio-gpu 2D (fase 11) |
| [0029](0029-usb-hid-sem-a-leitura-da-spec.md) | Driver de USB HID sem a leitura da especificação | Open | Antes de escrever o driver de USB HID (fase 8) |
| [0030](0030-intel-hda-sem-a-leitura-da-spec.md) | Driver de Intel HDA sem a leitura da especificação | Open | Antes de escrever o driver de Intel HDA, o que só acontece se o primeiro boot de áudio em metal precisar dele |
| [0032](0032-virtio-input-sem-a-leitura-da-spec.md) | virtio-input sem a leitura da especificação | Open | Antes de escrever o driver de virtio-input, na fase 6 |
| [0034](0034-tsc-deadline-sem-teste-no-x86.md) | TSC-deadline sem teste no x86_64 | Open | O primeiro boot do x86_64 com KVM ou em metal |
| [0035](0035-sve-sme-e-amx-sem-estado-por-tarefa.md) | SVE, SME e AMX sem estado por tarefa | Open | O primeiro código de usuário (fase 4), ou a primeira CPU de teste com SVE, SME ou AMX |
| [0041](0041-escalonador-do-x86-sem-medida-fora-do-tcg.md) | Escalonador do x86_64 sem medida fora do TCG | Open | O primeiro boot do x86_64 com KVM ou em metal |
| [0042](0042-troca-de-contexto-lenta-com-eevdf-e-relogio.md) | Troca de contexto com o EEVDF gasta metade do tempo na política | Open | O primeiro benchmark da troca de contexto contra o Linux (lat_ctx) no mesmo hardware |
| [0043](0043-simd-sem-trap-de-uso-indevido.md) | Tarefa sem SIMD que usa SIMD não é pega | Open | O primeiro código de usuário (fase 4), quando todo processo usa SSE |
| [0046](0046-teste-de-simd-sem-os-caminhos-de-xsave.md) | Autoteste de SIMD sem os caminhos de XSAVE e sem a metade alta | Open | A primeira CPU sem `XSAVEOPT`, ou o primeiro boot em metal |
| [0047](0047-timer-e-pilha-de-falha-sem-teste.md) | `arm_timer`, `next_timer_ns` e a pilha de falha sem teste | Open | O primeiro boot em metal, ou o primeiro defeito de timer ou de pilha de falha |
| [0048](0048-dec-sem-simbolos-de-controle-nem-nrcs.md) | DEC Special Graphics sem os símbolos de controle, e sem NRCS, G2 nem G3 | Open | O primeiro programa curses da fase 5 que desenhe ACS fora das caixas e dos blocos |
| [0049](0049-tempo-real-nao-migra-por-balanceamento.md) | Tempo real não migra por balanceamento entre CPUs | Open | A primeira tarefa de tempo real do kernel (a classe FIFO hoje não tem uso), ou o primeiro RT que espere atrás de outro numa CPU com outra ociosa |
| [0050](0050-historico-sem-rolagem-no-aarch64.md) | Histórico de rolagem desligado no aarch64 | Open | O primeiro teclado no aarch64 (virtio-input, fase 8) ou um atalho de rolagem pela serial |
| [0051](0051-redesenho-da-tela-com-irq-mascarada.md) | Redesenho da tela com a IRQ mascarada | Open | O primeiro benchmark de latência de IRQ ou de wakeup com a tela ligada, ou o primeiro código de tempo real que divida a CPU com o shell |
| [0052](0052-afinidade-so-no-spawn.md) | Afinidade por tarefa só no `spawn` | Open | A primeira syscall de afinidade (`sched_setaffinity`), na fase 4 |
| [0053](0053-balanceamento-sem-dominios.md) | Balanceamento de carga sem domínios, peso nem topologia | Open | O primeiro boot em máquina com SMT, vários caches ou NUMA, ou a primeira carga em que o número de tarefas por CPU não mede a carga (tarefas de pesos muito diferentes) |
| [0054](0054-heap-com-lock-global.md) | Heap do kernel com um lock global | Open | O primeiro programa que aloque do heap em todos os núcleos ao mesmo tempo (fase 4), ou a disputa do heap passar de 10% do tempo de uma CPU no `bench` |
| [0055](0055-kernel-sem-tlb-shootdown.md) | Kernel sem TLB shootdown entre núcleos | Open | O primeiro mapeamento do kernel que seja removido ou tenha permissão alterada depois do boot (pilha liberada, `munmap`, W^X dinâmico) |
| [0056](0056-irqs-de-dispositivo-so-na-cpu-de-boot.md) | IRQs de dispositivo roteadas só para a CPU de boot | Open | O primeiro dispositivo com taxa de interrupção que a CPU de boot não aguente, ou o primeiro driver com fila por núcleo (rede, disco) |
| [0057](0057-tsc-entre-nucleos-sem-verificacao.md) | TSC e contadores entre núcleos supostos sincronizados | Open | O primeiro boot do x86_64 em metal ou com KVM (o mesmo de DEBT-0034) |
| [0058](0058-zerar-pagina-no-x86-sem-medida-em-hardware.md) | Zerar página no x86_64 sem medida em hardware, `rep stosb` com ERMS fora | Open | O primeiro boot do x86_64 com KVM ou em metal |
| [0060](0060-recuo-do-zerar-sem-cobertura-no-smoke.md) | Recuo de STP do zerar página sem cobertura automática | Open | A primeira CPU aarch64 com DCZID_EL0.DZP ligado, ou o primeiro modelo da QEMU que o ligue |
| [0061](0061-percpu-sem-separar-linhas-de-cache.md) | PerCpu sem separar as linhas de cache por quem escreve | Open | O primeiro boot em metal, ou em VM com mais de 8 núcleos, em que a troca entre CPUs ou a fila por CPU meça pior que no HVF |
| [0062](0062-panico-conta-cpus-paradas-por-numero.md) | O pânico conta as CPUs paradas por número, não por máscara | Open | O primeiro pânico durante a subida dos núcleos que saia com texto de outra CPU intercalado, ou o primeiro boot em metal com mais de 4 núcleos |
| [0063](0063-nucleo-secundario-sem-vetores-nem-serial-precoce.md) | Núcleo secundário sem vetores nem serial precoce até o init próprio | Open | O primeiro boot que trave com `cpus: N online` abaixo do pedido e nenhuma linha de falha do núcleo que não subiu, ou o primeiro boot em metal com mais de um núcleo |
| [0064](0064-serial-precoce-so-confia-no-firmware.md) | Serial precoce confia no que o firmware deixou, só serial e só a da QEMU virt no aarch64 | Open | O primeiro boot em metal, ou o primeiro boot sem serial (só tela) |
| [0065](0065-acordar-com-lock-preso-troca-so-na-proxima-irq.md) | Acordar com `IrqLock` preso, ou com a IRQ mascarada pelo chamador, só troca na próxima IRQ | Open | O primeiro chamador de `unpark` ou `spawn` que segure um `IrqLock` ou mascare a IRQ e dependa da troca na hora |
| [0066](0066-calibracao-do-pit-com-janela-curta-sem-medida-em-kvm.md) | Calibração do TSC pelo PIT por latch, medida só em TCG | Open | O primeiro boot do x86_64 com KVM ou em metal |
| [0067](0067-log-do-boot-normal-sem-medida-em-uart-real.md) | Log do boot normal sem medida em UART física | Open | O primeiro boot em metal, ou em qualquer máquina com UART física lenta |
| [0068](0068-boot-de-teste-do-x86-em-tcg-bimodal-por-layout.md) | Tempo do boot de teste do x86_64 em TCG varia até 10x com o tamanho e o alinhamento do código | Open | O `make smoke` do x86_64 passar de 10 s, ou o CI do x86_64 estourar o tempo do smoke |
| [0070](0070-pilha-de-16-kib-sem-syscall-nem-falta-de-usuario.md) | Pilha de 16 KiB remedida só com syscall e falta fatal de usuário, sem handles, canais nem falta resolvida | Open | A 4c (handles, canais, VMAR) ou a 4f (falta de página que zera) rodarem na pilha do kernel de uma thread |
| [0072](0072-pilhas-de-excecao-sem-medida-permanente.md) | Pilhas de exceção (IST e de falhas) sem medida permanente de pico | Open | A 4f (medidas da fase), ou o primeiro `#DF`, NMI, `#MC` ou `#DB` real fora do autoteste |
| [0073](0073-limpeza-da-pilha-so-ate-128-bytes-de-limpo.md) | Limpeza da pilha do kernel na volta ao usuário para na primeira faixa limpa de 128 bytes | Open | O primeiro quadro de kernel no caminho de uma syscall ou falta de usuário com mais de 128 bytes de locais sem escrever, ou a medida da fase 4f em metal |
| [0074](0074-elf-recusa-osabi-gnu-do-used.md) | O validador de ELF recusa `EI_OSABI` diferente de 0, e o `rust-lld` grava 3 com `#[used]` | Open | O primeiro programa de usuário (ou o `user/rt`) que precise de `#[used]`, ou o carregador da fase 5 receber ELF de terceiros |
| [0075](0075-vmar-com-capacidade-fixa-de-regioes.md) | VMAR com capacidade fixa de regiões e deslocamento O(N) | Open | O primeiro processo que passar da capacidade escolhida pela 4b, ou a medida de `reserve`/`unmap` aparecer no perfil de criação de processo da 4f |
| [0076](0076-asid-sem-caminho-rapido-sem-lock.md) | Gerador de ASID/PCID sem caminho rápido sem lock | Open | A medida de troca entre processos da 4e/4f sob HVF/KVM mostrar o lock do gerador |
| [0077](0077-iretq-sem-fixup-de-falha-fora-do-rip-e-do-rsp.md) | Falha do `iretq` por outra causa que `RIP` ou `RSP` não canônico não tem fixup | Open | A 4e ligar o KPTI (o `iretq` passa a rodar com o CR3 do usuário), ou a primeira falha de `iretq` que a conferência de `RIP` e `RSP` canônicos não pegue |
| [0078](0078-aarch64-excecao-do-el1h-na-pilha-de-falhas.md) | Exceção síncrona de EL1h do aarch64 segue na pilha de falhas, sem o teste de estouro da pilha da tarefa | Open | A primeira falta de página de usuário resolvida dentro do kernel (a 4f: página zero e COW), ou o primeiro `brk` ou *fixup* que precise reescalonar |
| [0081](0081-debug-write-de-512-bytes-com-a-irq-mascarada.md) | `debug_write` de até 512 bytes roda com a IRQ mascarada e o lock do console preso | Open | A 4c ligar a IRQ no despacho, ou o primeiro boot em UART física |
| [0082](0082-sem-lfence-depois-do-swapgs-condicional.md) | Sem `lfence` depois do `swapgs` condicional das entradas do x86_64 | Open | O primeiro boot do x86_64 em metal ou com KVM, ou a 4e (mitigações de CPU) |
