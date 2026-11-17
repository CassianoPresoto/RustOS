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
| [0002](0002-pl011-no-endereco-da-qemu-virt.md) | Endereço da PL011 fixo no da QEMU `virt` | Open | A primeira placa aarch64 que não seja a QEMU `virt` |
| [0003](0003-spinlock-sem-mascarar-interrupcao.md) | `SpinLock` não mascara interrupção | Open | O primeiro handler de interrupção que pegue um lock, inclusive o do console |
| [0004](0004-x86-sem-mapeador-de-pagina.md) | x86_64 sem `PageMapper`; janela de dispositivos só no aarch64 | Open | O primeiro registrador MMIO no x86_64 (Local APIC, fase 1) |
| [0005](0005-panico-antes-do-console-e-mudo.md) | Pânico antes do console não aparece | Open | O primeiro boot que trave sem saída na serial, ou o primeiro boot em metal |
| [0006](0006-memoria-do-bootloader-nao-recuperada.md) | Memória do bootloader não volta para o alocador | Open | O primeiro benchmark de memória ociosa contra o Linux |
