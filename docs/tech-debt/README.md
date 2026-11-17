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
| [0008](0008-terminal-sem-tela-alternativa.md) | Terminal sem tela alternativa, rolagem para trás nem caractere largo | Open | O primeiro programa de tela cheia (vi, less, top) rodando no terminal, na fase 5 |
| [0009](0009-ioapic-no-endereco-fixo-da-qemu.md) | I/O APIC no endereço fixo e IRQs ISA sem redirecionamento do MADT | Open | A primeira máquina x86_64 que não seja a QEMU q35, ou o parser de ACPI da fase 6 |
| [0010](0010-teclado-sem-comandos-ao-dispositivo.md) | Teclado sem comandos ao dispositivo, LEDs nem Num Lock | Open | O primeiro boot em metal com um teclado PS/2 real |
| [0011](0011-gic-e-timer-so-na-cpu-0.md) | GIC e timer configurados só para a CPU 0 | Open | O primeiro núcleo secundário subir, na fase 3 |
| [0012](0012-excecao-de-usuario-so-para-a-cpu.md) | Exceção vinda de código de usuário só para a CPU | Open | O primeiro código em anel 3 ou EL0, na fase 4 |
| [0014](0014-x86-sem-salvaguardas-de-metal.md) | x86_64 sem as salvaguardas que só o metal pede | Open | O primeiro boot do x86_64 em metal |
| [0015](0015-cripto-sem-medida-no-x86-acelerado.md) | Criptografia do núcleo sem medida do x86_64 com aceleração | Open | O primeiro boot do x86_64 com KVM ou em metal |
| [0017](0017-dupla-liberacao-escondida-no-cache-de-frames.md) | Dupla liberação de frame no cache por CPU só aparece ao esvaziar o cache | Open | O primeiro defeito de dupla liberação de frame que o cache esconder, ou o modo de depuração do kernel |
| [0018](0018-frames-sem-cache-por-cpu.md) | Alocador de frames sem o cache por CPU | Open | O primeiro núcleo secundário subir, na fase 3 |
| [0019](0019-aleatorio-sem-fonte-de-hardware-fraco.md) | Aleatório só com jitter do contador em CPU sem gerador de hardware | Open | O primeiro consumidor de segurança do aleatório (KASLR, canário, ASLR de usuário) rodando em CPU sem RDSEED, RDRAND nem RNDR |
| [0020](0020-zerar-pagina-pelo-memset-generico.md) | Zerar memória ao liberar usa o memset genérico, 5x mais lento que o do host | Open | O primeiro caminho quente que zere página inteira (page fault de usuário, na fase 4), ou o primeiro benchmark de fork, exec ou mmap contra o Linux |
| [0021](0021-mapa-direto-sem-acpi-nem-reservada-mapeada.md) | Mapa direto sem as tabelas ACPI nem as regiões reservadas que o Limine mapeia | Open | O primeiro código que lê ACPI (MADT, MCFG) ou outra região reservada mapeada, na fase 6 |
| [0022](0022-endereco-do-kernel-no-log-de-boot.md) | Endereços do kernel e do mapa direto impressos no boot, no fault e no pânico | Open | O primeiro código de usuário (fase 4) ou o primeiro log que saia da máquina |
| [0023](0023-aarch64-sem-wxn.md) | aarch64 sem SCTLR_EL1.WXN | Open | O primeiro código de usuário (fase 4), ou o primeiro mapeamento criado depois do boot que possa ser gravável e executável |
| [0024](0024-aarch64-pie-com-textrel.md) | Kernel aarch64 relocável só com `-z notext` (TEXTREL) | Open | A `core` pré-compilada do alvo aarch64 passar a vir com relocação PIC, ou o primeiro build com build-std |
| [0025](0025-residuo-de-chave-na-pilha.md) | Cópias do estado do ChaCha20 e do gerador antigo ficam na pilha | Open | O primeiro código de usuário (fase 4) |
| [0026](0026-troca-de-tabela-do-aarch64-sem-conferencia.md) | Troca de tabela do aarch64 sem a leitura do Arm ARM nem a conferência do MAIR | Open | O primeiro boot em metal aarch64, ou a leitura do Arm ARM nas seções de TLB e de MAIR |
| [0027](0027-virtio-snd-sem-a-leitura-da-spec.md) | Driver de virtio-snd sem a leitura da especificação | Open | Antes de escrever o driver de virtio-snd (fase 6) |
| [0028](0028-virtio-gpu-2d-sem-a-leitura-da-spec.md) | Driver de virtio-gpu 2D sem a leitura da especificação | Open | Antes de escrever o driver de virtio-gpu 2D (fase 11) |
| [0029](0029-usb-hid-sem-a-leitura-da-spec.md) | Driver de USB HID sem a leitura da especificação | Open | Antes de escrever o driver de USB HID (fase 8) |
| [0030](0030-intel-hda-sem-a-leitura-da-spec.md) | Driver de Intel HDA sem a leitura da especificação | Open | Antes de escrever o driver de Intel HDA, o que só acontece se o primeiro boot de áudio em metal precisar dele |
| [0031](0031-memoria-ociosa-sem-linha-de-base.md) | Alvo de memória ociosa sem linha de base medida | Open | Antes de fechar a fase 5 |
| [0032](0032-virtio-input-sem-a-leitura-da-spec.md) | virtio-input sem a leitura da especificação | Open | Antes de escrever o driver de virtio-input, na fase 6 |
| [0034](0034-tsc-deadline-sem-teste-no-x86.md) | TSC-deadline sem teste no x86_64 | Open | O primeiro boot do x86_64 com KVM ou em metal |
| [0035](0035-sve-sme-e-amx-sem-estado-por-tarefa.md) | SVE, SME e AMX sem estado por tarefa | Open | O primeiro código de usuário (fase 4), ou a primeira CPU de teste com SVE, SME ou AMX |
| [0036](0036-escalonador-com-estado-de-troca-global.md) | Escalonador com estado de troca global | Open | O primeiro núcleo secundário subir, na fase 3 |
| [0037](0037-pilha-de-tarefa-sem-medida-de-pico.md) | Pilha de tarefa de 32 KiB sem medida de pico | Open | O primeiro estouro de pilha de uma tarefa, ou o primeiro código de usuário (fase 4) |
| [0039](0039-escalonador-de-uma-cpu.md) | Escalonador de uma CPU, sem fila por CPU nem balanceamento | Open | O primeiro núcleo secundário rodar tarefas, na fase 3 |
| [0041](0041-escalonador-do-x86-sem-medida-fora-do-tcg.md) | Escalonador do x86_64 sem medida fora do TCG | Open | O primeiro boot do x86_64 com KVM ou em metal |
| [0042](0042-troca-de-contexto-lenta-com-eevdf-e-relogio.md) | Troca de contexto com o EEVDF custa o dobro da do rodízio | Open | O primeiro benchmark da troca de contexto contra o Linux (lat_ctx) no mesmo hardware |
