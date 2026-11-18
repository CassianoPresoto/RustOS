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
| [0005](0005-panico-antes-do-console-e-mudo.md) | Pânico antes do console, ou com o console preso, não aparece | Open | O primeiro boot que trave sem saída na serial, ou o primeiro boot em metal |
| [0008](0008-terminal-sem-caractere-largo.md) | Terminal sem caractere largo nem combinante | Open | O primeiro programa da fase 5 (vi, less) mostrando CJK, emoji ou acento decomposto no terminal, ou a fonte ganhar glifo de largura dupla |
| [0009](0009-ioapic-no-endereco-fixo-da-qemu.md) | I/O APIC no endereço fixo e IRQs ISA sem redirecionamento do MADT | Open | A primeira máquina x86_64 que não seja a QEMU q35, ou o parser de ACPI da fase 6 |
| [0010](0010-teclado-sem-comandos-ao-dispositivo.md) | Teclado sem comandos ao dispositivo, LEDs nem Num Lock | Open | O primeiro boot em metal com um teclado PS/2 real |
| [0012](0012-excecao-de-usuario-so-para-a-cpu.md) | Exceção vinda de código de usuário só para a CPU | Open | O primeiro código em anel 3 ou EL0, na fase 4 |
| [0014](0014-x86-sem-salvaguardas-de-metal.md) | x86_64 sem as salvaguardas que só o metal pede | Open | O primeiro boot do x86_64 em metal |
| [0015](0015-cripto-sem-medida-no-x86-acelerado.md) | Criptografia do núcleo sem medida do x86_64 com aceleração | Open | O primeiro boot do x86_64 com KVM ou em metal |
| [0017](0017-dupla-liberacao-escondida-no-cache-de-frames.md) | Dupla liberação de frame no cache por CPU só aparece ao esvaziar o cache | Open | O primeiro defeito de dupla liberação de frame que o cache esconder, ou o modo de depuração do kernel |
| [0019](0019-aleatorio-sem-fonte-de-hardware-fraco.md) | Aleatório só com jitter do contador em CPU sem gerador de hardware | Open | O primeiro consumidor de segurança do aleatório (KASLR, canário, ASLR de usuário) rodando em CPU sem RDSEED, RDRAND nem RNDR |
| [0021](0021-mapa-direto-sem-acpi-nem-reservada-mapeada.md) | Mapa direto sem as tabelas ACPI nem as regiões reservadas que o Limine mapeia | Open | O primeiro código que lê ACPI (MADT, MCFG) ou outra região reservada mapeada, na fase 6 |
| [0022](0022-endereco-do-kernel-no-log-de-boot.md) | Endereços do kernel e do mapa direto impressos no boot, no fault e no pânico | Open | O primeiro código de usuário (fase 4) ou o primeiro log que saia da máquina |
| [0023](0023-aarch64-sem-wxn.md) | aarch64 sem SCTLR_EL1.WXN | Open | O primeiro código de usuário (fase 4), ou o primeiro mapeamento criado depois do boot que possa ser gravável e executável |
| [0024](0024-aarch64-pie-com-textrel.md) | Kernel aarch64 relocável só com `-z notext` (TEXTREL) | Open | A `core` pré-compilada do alvo aarch64 passar a vir com relocação PIC, ou o primeiro build com build-std |
| [0026](0026-troca-de-tabela-do-aarch64-sem-conferencia.md) | Troca de tabela do aarch64 sem a leitura do Arm ARM nem a conferência do MAIR | Open | O primeiro boot em metal aarch64, ou a leitura do Arm ARM nas seções de TLB e de MAIR |
| [0027](0027-virtio-snd-sem-a-leitura-da-spec.md) | Driver de virtio-snd sem a leitura da especificação | Open | Antes de escrever o driver de virtio-snd (fase 6) |
| [0028](0028-virtio-gpu-2d-sem-a-leitura-da-spec.md) | Driver de virtio-gpu 2D sem a leitura da especificação | Open | Antes de escrever o driver de virtio-gpu 2D (fase 11) |
| [0029](0029-usb-hid-sem-a-leitura-da-spec.md) | Driver de USB HID sem a leitura da especificação | Open | Antes de escrever o driver de USB HID (fase 8) |
| [0030](0030-intel-hda-sem-a-leitura-da-spec.md) | Driver de Intel HDA sem a leitura da especificação | Open | Antes de escrever o driver de Intel HDA, o que só acontece se o primeiro boot de áudio em metal precisar dele |
| [0032](0032-virtio-input-sem-a-leitura-da-spec.md) | virtio-input sem a leitura da especificação | Open | Antes de escrever o driver de virtio-input, na fase 6 |
| [0034](0034-tsc-deadline-sem-teste-no-x86.md) | TSC-deadline sem teste no x86_64 | Open | O primeiro boot do x86_64 com KVM ou em metal |
| [0035](0035-sve-sme-e-amx-sem-estado-por-tarefa.md) | SVE, SME e AMX sem estado por tarefa | Open | O primeiro código de usuário (fase 4), ou a primeira CPU de teste com SVE, SME ou AMX |
| [0037](0037-pilha-de-tarefa-sem-medida-de-pico.md) | Pilha de tarefa de 32 KiB sem medida de pico | Open | O primeiro estouro de pilha de uma tarefa, ou o primeiro código de usuário (fase 4) |
| [0041](0041-escalonador-do-x86-sem-medida-fora-do-tcg.md) | Escalonador do x86_64 sem medida fora do TCG | Open | O primeiro boot do x86_64 com KVM ou em metal |
| [0042](0042-troca-de-contexto-lenta-com-eevdf-e-relogio.md) | Troca de contexto com o EEVDF custa o dobro da do rodízio | Open | O primeiro benchmark da troca de contexto contra o Linux (lat_ctx) no mesmo hardware |
| [0043](0043-simd-sem-trap-de-uso-indevido.md) | Tarefa sem SIMD que usa SIMD não é pega | Open | O primeiro código de usuário (fase 4), quando todo processo usa SSE |
| [0044](0044-wake-de-tarefa-so-troca-na-proxima-irq.md) | Wake feito em contexto de tarefa só troca na próxima IRQ | Open | O primeiro programa interativo que medir a latência de acordar entre tarefas (áudio ou jogo) |
| [0045](0045-trava-presa-ao-ceder-a-cpu-some-em-release.md) | `debug_assert` de lock preso ao ceder a CPU some em `release` | Open | O primeiro travamento em `release` que não reproduza em `dev` |
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
| [0059](0059-estado-do-chacha20-derramado-na-pilha-no-x86.md) | Resíduo em `core/crypto` que o autoteste de pilha não enxerga | Open | A primeira volta do kernel para código de usuário (a syscall da fase 4), ou o primeiro `copy_to_user` que leia a pilha do kernel |
| [0060](0060-recuo-do-zerar-sem-cobertura-no-smoke.md) | Recuo de STP do zerar página sem cobertura automática | Open | A primeira CPU aarch64 com DCZID_EL0.DZP ligado, ou o primeiro modelo da QEMU que o ligue |
| [0061](0061-percpu-sem-separar-linhas-de-cache.md) | PerCpu sem separar as linhas de cache por quem escreve | Open | O primeiro boot em metal, ou em VM com mais de 8 núcleos, em que a troca entre CPUs ou a fila por CPU meça pior que no HVF |
