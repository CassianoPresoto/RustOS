---
id: DEBT-0080
title: Os frames do módulo initramfs ficam residentes, sem voltar ao buddy
status: Open
date: 2026-11-18
trigger: A 4c criar o VMO do initramfs (o ponto em que o último handle fechado deveria devolver os frames), ou um initramfs de mais de 1 MiB
paid_date:
---

# DEBT 0080 — Os frames do módulo initramfs ficam residentes, sem voltar ao buddy

## O que falta

O Limine carrega o módulo em memória marcada `KernelAndModules`, e o kernel só a mapeia (só leitura, NX, na janela de módulos). `Memory::init` só entrega ao buddy as regiões `Usable` e `reclaim_bootloader` as `BootloaderReclaimable`; as do módulo nunca voltam. O initramfs custa, para sempre, o tamanho dele arredondado a 4 KiB (272 bytes de cpio provisório hoje: uma página).

## Por que foi aceito

O PROTOCOL.md (`limine-protocol`, commit `877c2d4`, o que o `bootstrap` do Limine 12.10.0 fixa) diz que as entradas `Executable and modules` "são ilustrativas" e **não** garante alinhamento nem que não se sobreponham a outras; a seção *File Structure* garante só que cada módulo tem "todos os pedaços de 4 KiB que cobre, para si". Nada diz que o kernel pode reusar essa memória. Que possa é **NÃO VERIFICADO** (SDD 17). Para o `init` de poucas dezenas de KiB o custo é desprezível.

## Gatilho

A 4c criar o VMO físico do initramfs (devolver os frames ao fechar o último handle é parte do desenho do SDD 17), ou o initramfs passar de 1 MiB, o que torna a residência visível em `mem`. A imagem de **teste** da 4b tem 1,3 MiB por causa do programa `big`; isso não dispara o gatilho, que fala da imagem de produção (o `init` tem cerca de 17 KiB).

## Como pagar

Ler no `PROTOCOL.md` e no `limine.c` (função `uri_open` com `MEMMAP_KERNEL_AND_MODULES`) se a memória do módulo pode virar `Usable` depois de lida; se pode, `release_range` das páginas inteiras do módulo quando o VMO morrer (o `BootRecord` já guarda `start` e `len`); se não pode, copiar o initramfs para frames do alocador na entrada e liberar o original só no que o protocolo permitir, ou aceitar a residência e fechar esta dívida como `Dropped`. Mexe em `kernel/src/vm.rs`, `kernel/src/memory.rs` e no SDD 13.
