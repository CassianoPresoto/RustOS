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
| [0006](0006-memoria-do-bootloader-nao-recuperada.md) | Memória do bootloader não volta para o alocador | Open | O primeiro benchmark de memória ociosa contra o Linux |
| [0008](0008-terminal-sem-tela-alternativa.md) | Terminal sem tela alternativa, rolagem para trás nem caractere largo | Open | O primeiro programa de tela cheia (vi, less, top) rodando no terminal, na fase 5 |
| [0009](0009-ioapic-no-endereco-fixo-da-qemu.md) | I/O APIC no endereço fixo e IRQs ISA sem redirecionamento do MADT | Open | A primeira máquina x86_64 que não seja a QEMU q35, ou o parser de ACPI da fase 6 |
| [0010](0010-teclado-sem-comandos-ao-dispositivo.md) | Teclado sem comandos ao dispositivo, LEDs nem Num Lock | Open | O primeiro boot em metal com um teclado PS/2 real |
| [0011](0011-gic-e-timer-so-na-cpu-0.md) | GIC e timer configurados só para a CPU 0 | Open | O primeiro núcleo secundário subir, na fase 3 |
| [0012](0012-excecao-de-usuario-so-para-a-cpu.md) | Exceção vinda de código de usuário só para a CPU | Open | O primeiro código em anel 3 ou EL0, na fase 4 |
| [0013](0013-caminhada-de-tabela-duplicada.md) | Caminhada de tabela de página duplicada entre as ISAs | Open | A terceira ISA, ou a primeira mudança no algoritmo de caminhada de uma delas |
| [0014](0014-x86-sem-salvaguardas-de-metal.md) | x86_64 sem as salvaguardas que só o metal pede | Open | O primeiro boot do x86_64 em metal |
| [0015](0015-cripto-sem-medida-no-alvo.md) | Criptografia do núcleo medida só no host | Open | O kernel ligar o `Rng` de `core/crypto`, na fase 2 |
| [0016](0016-bitmap-de-frames-ainda-no-kernel.md) | Kernel ainda usa o BitmapFrameAllocator; o buddy e o heap não estão ligados | Open | O PR que liga o heap do kernel (GlobalAlloc) na fase 2 |
| [0017](0017-dupla-liberacao-escondida-no-cache-de-frames.md) | Dupla liberação de frame no cache por CPU só aparece ao esvaziar o cache | Open | O primeiro defeito de dupla liberação de frame que o cache esconder, ou o modo de depuração do kernel |
