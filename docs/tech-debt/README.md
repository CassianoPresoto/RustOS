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
| [0013](0013-caminhada-de-tabela-duplicada.md) | Caminhada de tabela de página duplicada entre as ISAs | Open | A terceira ISA, ou a primeira mudança no algoritmo de caminhada de uma delas |
| [0014](0014-x86-sem-salvaguardas-de-metal.md) | x86_64 sem as salvaguardas que só o metal pede | Open | O primeiro boot do x86_64 em metal |
| [0015](0015-cripto-sem-medida-no-x86-acelerado.md) | Criptografia do núcleo sem medida do x86_64 com aceleração | Open | O primeiro boot do x86_64 com KVM ou em metal |
| [0017](0017-dupla-liberacao-escondida-no-cache-de-frames.md) | Dupla liberação de frame no cache por CPU só aparece ao esvaziar o cache | Open | O primeiro defeito de dupla liberação de frame que o cache esconder, ou o modo de depuração do kernel |
| [0018](0018-frames-sem-cache-por-cpu.md) | Alocador de frames sem o cache por CPU | Open | O primeiro núcleo secundário subir, na fase 3 |
| [0019](0019-aleatorio-sem-fonte-de-hardware-fraco.md) | Aleatório só com jitter do contador em CPU sem gerador de hardware | Open | O primeiro consumidor de segurança do aleatório (KASLR, canário, ASLR de usuário) rodando em CPU sem RDSEED, RDRAND nem RNDR |
| [0020](0020-zerar-pagina-pelo-memset-generico.md) | Zerar memória ao liberar usa o memset genérico, 5x mais lento que o do host | Open | O primeiro caminho quente que zere página inteira (page fault de usuário, na fase 4), ou o primeiro benchmark de fork, exec ou mmap contra o Linux |
| [0021](0021-mapa-direto-sem-acpi-nem-reservada-mapeada.md) | Mapa direto sem as tabelas ACPI nem as regiões reservadas que o Limine mapeia | Open | O primeiro código que lê ACPI (MADT, MCFG) ou outra região reservada mapeada, na fase 6 |
| [0022](0022-endereco-do-kernel-no-log-de-boot.md) | Endereço-base do kernel e do mapa direto impressos no boot | Open | O primeiro código de usuário (fase 4) ou o primeiro log que saia da máquina |
| [0023](0023-aarch64-sem-wxn.md) | aarch64 sem SCTLR_EL1.WXN | Open | O primeiro código de usuário (fase 4), ou o primeiro mapeamento criado depois do boot que possa ser gravável e executável |
| [0024](0024-aarch64-pie-com-textrel.md) | Kernel aarch64 relocável só com `-z notext` (TEXTREL) | Open | A `core` pré-compilada do alvo aarch64 passar a vir com relocação PIC, ou o primeiro build com build-std |
