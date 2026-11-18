# SDD 13 — Memória virtual: tabelas próprias, W^X e KASLR

> **O que este capítulo responde:** como o kernel monta o próprio espaço de
> endereçamento, que permissão cada faixa tem e por quê, como troca de pilha e
> devolve a memória do bootloader, e o que o KASLR custa.

Fase 2 do [roadmap](08_roadmap.md). A ordem no boot está no [SDD 04](04_boot.md);
a memória física e o heap, no [SDD 12](12_memoria_fisica_e_heap.md).

## O que o `kmain` faz, em ordem

1. Lê as respostas do Limine (`LimineBoot`), sobe o buddy e o heap (`Memory::init`).
2. **Copia** o que ainda lê do Limine para um `BootRecord` no heap (`kernel/src/boot.rs`):
   mapa de memória, deslocamento do mapa direto, nome e versão do bootloader, marcos
   de tempo, framebuffer, endereço da imagem e módulos (faixa física e `string`). Depois disso nada lê a resposta do Limine.
3. `platform::harden()` liga as proteções da CPU (x86_64: WP, NX, SMEP, SMAP, UMIP;
   aarch64: PAN; `SCTLR_EL1.WXN`, ligado depois de `Vm::activate` em cada CPU, vale também para EL0). No x86_64 o NX precisa estar no `EFER` antes de a tabela usar o bit.
4. `Vm::build` monta tabelas novas, sem ativá-las. `Memory::place_windows` põe as janelas
   de dispositivos, de pilhas e de módulos. `Vm::map_initramfs` mapeia o initramfs
   ([abaixo](#o-módulo-initramfs)). `Vm::activate` troca `CR3` / `TTBR1_EL1`.
5. `Memory::map_stack` cria a pilha do kernel (64 KiB e uma página de guarda) e
   `platform::call_on_stack` passa a rodar nela (`asm!` em `arch/*/src/stack.rs`),
   entregando o `Handoff` (registro de boot, memória e `Vm`) por ponteiro.
6. Na pilha nova: console, tela, interrupções (a GDT própria do x86_64 entra aqui) e
   `Memory::reclaim_bootloader`, que devolve ao buddy as regiões `BootloaderReclaimable`.
7. Relatório, autotestes, `w^x: ok` e o shell.

Pânico e exceção antes do console saem direto na serial ([SDD 04](04_boot.md#pânico-e-exceção-antes-do-console)); no aarch64 a PL011 é mapeada no espaço novo antes de `activate`.

## Layout virtual

| Faixa | O quê | Permissão | Quem escolhe |
| --- | --- | --- | --- |
| Metade baixa | A raiz do kernel nada tem; cada processo tem a sua | x86_64: a raiz do kernel tem as 256 entradas de baixo vazias, e a de um processo copia as 256 de cima por ponteiro (conferido em `Process::load`). aarch64: `TTBR0_EL1` aponta uma raiz vazia nas tarefas de kernel e a do processo nas de usuário, com `EPD0` limpo desde a 4a | Folha de usuário só em `[USER_MIN, USER_TOP)` e só com `PageFlags.user`; o mapeador recusa o resto ([SDD 17](17_processos_isolamento_e_syscalls.md#a-fatia-4a-como-entrou)) |
| `[__image_start, __requests_end)` | `.requests`, `.dynamic`, `.got` | leitura, NX | Limine (KASLR), tamanho pelo linker script |
| `[__text_start, __text_end)` | `.text` | leitura e execução | idem |
| `[__rodata_start, __rodata_end)` | `.rodata`, `.dynsym`, `.rela.dyn`, `.data.rel.ro` | leitura, NX | idem |
| `[__data_start, __data_end)` | `.data`, `.bss` | leitura e escrita, NX | idem |
| `hhdm + [início, fim)` | RAM: `Usable` e `BootloaderReclaimable` | leitura e escrita, NX, write-back | Limine sorteia o deslocamento |
| `hhdm +` framebuffer | O framebuffer da resposta, páginas inteiras | leitura e escrita, NX, write-combining | idem |
| 1 entrada do nível mais alto | Janela de dispositivos (1 GiB), janela de pilhas (1 GiB) e janela de módulos (1 GiB), nessa ordem | dados, NX; dispositivo sem cache; módulo só leitura, NX | kernel: a mais alta entre 510 e 256 que o mapa direto e a imagem não tocam (`free_top_level_slot`) |

Os símbolos `__*_start/_end` vêm de `kernel/linker/<isa>.ld`, alinhados a 4 KiB. O
`Vm::build` recusa o boot se `__image_start` não for o `virtual_base` que o Limine
informou.

**O mapa direto não cobre a imagem do kernel.** A região `KernelAndModules` fica fora do
mapa direto: um alias gravável de `.text` anularia a proteção. O autoteste confere isso.
**Nem dispositivo, nem ACPI** ([DEBT-0021](../tech-debt/0021-mapa-direto-sem-acpi-nem-reservada-mapeada.md)).

### O módulo initramfs

O módulo que o Limine carrega cai em `KernelAndModules` e, por isso, **fora do mapa direto**
(só `Usable` e `BootloaderReclaimable` entram nele). `Vm::map_initramfs` o mapeia na
**janela de módulos**, a terceira de 1 GiB da mesma entrada do nível mais alto (a que o
mapa direto e a imagem não tocam): `memory::map_module` toma as páginas da janela e mapeia
uma a uma com `PageFlags::RODATA` (leitura, NX), em páginas de 4 KiB. A faixa física vem do
adaptador do Limine, já conferida ([SDD 04](04_boot.md#o-que-o-kernel-pede): alinhada, dentro do mapa de memória, fora de RAM do alocador, do framebuffer e de outro módulo), e o `Vm` recusa a que toca a imagem. O mapeamento é `rustos_mm::map_module` (`core/mm/src/window.rs`, testado no host: arredondamento a páginas, janela esgotada, estouro); o
`WxChecker` a vê como qualquer folha: se ela fosse gravável e executável, ou se tocasse a
memória física da imagem (`ImageAlias`), o `w^x:` reprovaria. No modo de teste,
`Vm::initramfs_is_read_only` confere que nenhuma folha do módulo é gravável ou executável e
que o número de páginas bate com o tamanho. A janela não tem região própria no `Addr`
(SDD 09): o endereço dela sairia como `<kernel>`, e o kernel não o imprime.
Custo para o cpio de 272 bytes (uma página), com `now_ns` depois da calibração e um laço de
64 mapeamentos da mesma página em endereços novos da janela (instrumentação temporária, não
mantida): no x86_64 em TCG a primeira chamada leva 132 a 141 µs (3 boots; domina a primeira
execução do código no TCG, não a tabela) e 0,36 a 0,42 µs por página depois; no aarch64 com HVF, 0,17 µs
a primeira e 33 ns por página. A chamada real do `kmain`, que ainda cria os frames de tabela, mediu 86 000 ciclos
de TSC no x86_64 (o TSC não está calibrado ali; **estimado** em 80 µs pela razão com os 4,3 milhões de
ciclos do `kmain` até esse ponto, cerca de 4 ms) e 157 ticks do `CNTVCT_EL0` (cerca de 7 µs) no aarch64.
Ordem de grandeza: de 0,08 a 0,15 ms em TCG, que some no ruído do `boot time:` (24 a 32 ms).

Os frames do módulo **seguem reservados**: nunca vão ao buddy ([DEBT-0080](../tech-debt/0080-initramfs-residente-sem-devolver-os-frames.md)).

## Flags e codificação

`hal::PageFlags` tem `CODE`, `RODATA`, `DATA`, `DEVICE` e `FRAMEBUFFER`
(`core/hal/src/paging.rs`). A codificação mora na parte pura de cada `arch/`, com
teste no host que confere ida e volta (`decode_leaf(page_entry(f)) == f`).

| Flag | x86_64 (`pte.rs`) | aarch64 (`descriptor.rs`) |
| --- | --- | --- |
| `CODE` | P, A, sem W, sem NX | `AP[2]` (só leitura), PXN limpo |
| `RODATA` | P, A, sem W, NX | `AP[2]`, PXN |
| `DATA` | P, A, W, D, NX | PXN, `AttrIndx` 0, inner shareable |
| `DEVICE` | como `DATA` com `PCD\|PWT` (entrada 3 do PAT, UC) | `AttrIndx` 7 (Device-nGnRnE, do kernel) |
| `FRAMEBUFFER` | como `DATA` com `PAT\|PWT` (entrada 5, WC) | `AttrIndx` 1 (o do framebuffer, do Limine), outer shareable |

Em todas as folhas do aarch64 o UXN fica ligado (página do kernel nunca executa em EL0).

- **Uma caminhada só.** `rustos_mm::PageTables<Fmt, Mem, Ctl>` (`core/mm/src/tables.rs`)
  desce os 4 níveis, cria tabelas zeradas, recusa folha em cima de folha e percorre
  as folhas com a permissão efetiva. Ela fala com três portas do `hal`: `PageFormat`
  (a codificação, em `pte.rs` e `descriptor.rs`), `TableMemory` (o mapa direto, em
  `mmu.rs`) e `MmuControl` (`invlpg` ou barreiras, e a troca de `CR3` ou `TTBR1_EL1`).
  Como `arch/` não depende de `core/mm` (guidelines 03), quem monta o espaço é
  `kernel/src/platform/`: `PlatformSpace = PageTables<X86Format, DirectMapTables, X86Mmu>`.
- **W^X na hora de mapear.** `map_page` e `map_large_page` devolvem
  `MapError::WriteExecute` para `PageFlags` gravável e executável, antes de tocar a
  tabela, nas duas ISAs. Os codificadores (`page_entry`, `page_descriptor`) seguem
  puros e não recusam: quem recusa é o mapeador.
- **x86_64, write-combining.** O Limine deixa `PAT[5] = WC` (PROTOCOL.md, *Caching*).
  `require_features` lê o `IA32_PAT` e recusa o boot se a entrada 5 não for
  WC (tipo `0x01`), se o NX não estiver ligado no `EFER` e se `CR4.PGE` estiver ligado
  (o `mov cr3` não descarta página global). O bit PAT é o 7 na folha de
  4 KiB e o 12 na de 2 MiB (`_PAGE_BIT_PAT` e `_PAGE_BIT_PAT_LARGE` em
  [`pgtable_types.h` do Linux](https://github.com/torvalds/linux/blob/master/arch/x86/include/asm/pgtable_types.h)).
- **aarch64, framebuffer.** `MAIR_EL1.Attr1` é "o do framebuffer, do tipo certo" a partir
  da revisão base 4 (PROTOCOL.md, *Caching*). O kernel não o reescreve; só escreve
  o índice 7.
- **Hierarquia.** A entrada de tabela é permissiva (x86_64: `P|W`; aarch64: sem
  `APTable`/`PXNTable`): quem restringe é a folha. A caminhada de conferência calcula a
  permissão **efetiva** (E da escrita, OU do NX entre os níveis), então uma tabela
  restritiva no meio também apareceria.

## Zerar

`DirectMap::zero` manda ao `arch/` as faixas de 512 bytes ou mais, com início e tamanho
múltiplos de 64 (`platform::zero_aligned`); o resto vai ao `write_bytes`. Números no
[SDD 07](07_desempenho_e_recursos.md).

- **aarch64** (`arch/aarch64/src/zero.rs`, divisão pura em `zero_plan.rs`). `DC ZVA` zera um
  bloco alinhado de `4 << DCZID_EL0.BS` bytes, e `DZP` (bit 4) ligado o proíbe. O
  `zero_aligned` lê o `DCZID_EL0` a cada chamada, como o Linux em cada `clear_page`: o
  registrador é por CPU, e um bloco lido na CPU 0 corromperia os vizinhos numa CPU de bloco
  maior; com `DZP` ligado, ou bloco menor que 64, o laço é de `STP` de pares (64 bytes por volta). O começo antes do primeiro bloco alinhado e
  a sobra depois do último vão por `STP`, o miolo por `DC ZVA`. No HVF o bloco é de 64 bytes e
  `DZP` está limpo.
- **Só vale em memória Normal.** O mapa direto é `DATA` com `AttrIndx` 0, e o `MAIR_EL1` lido
  em boot é `0xffff` (Attr0 = `0xff`, Normal write-back; o kernel só escreve o índice 7, de
  dispositivo). `DC ZVA` nunca vai para o framebuffer (índice 1) nem para a janela de
  dispositivos: `DirectMap` só toca frames do alocador.
- **x86_64.** `zero_aligned` é o `write_bytes`; ver o SDD 07 e a
  [DEBT-0058](../tech-debt/0058-zerar-pagina-no-x86-sem-medida-em-hardware.md).
- **Fontes.** Linux `arch/arm64/lib/clear_page.S` (v6.6 e v6.12, lido): `mrs dczid_el0`,
  `tbnz #4` para o recuo, `4 << (x & 0xf)`, e o recuo com `stnp`. Linux
  `arch/x86/lib/clear_page_64.S` e `page_64.h` (v6.12, lidos): `clear_page_orig`, `_rep`
  (`stosq`) e `_erms` (`stosb`). **Não verificado no Arm ARM** (a página da Arm não é
  legível por busca automática): que `DC ZVA` em memória Device ou não cacheável dá falta de
  alinhamento, e os campos `DZP` e `BS` além do que o código do Linux usa.
- **`STP`, não `STNP`, no recuo.** O Linux usa `STNP` (não temporal). Aqui o recuo zera
  memória que o heap reutiliza logo, então o dado deve ficar no cache. Medido no HVF
  com `DZP` forçado, `release`: `STP` 4 KiB em 39 a 41 ns e 1 KiB em 11 a 12; `STNP`, 38 a 82
  e 15 a 18. `STNP` não ganhou, e varia mais.
- **Teste.** Host: `zero_plan` (bloco por `DCZID`, `DZP`, divisão em cabeça, corpo e cauda para
  vários blocos e alinhamentos). No alvo: o autoteste do heap confere que os vizinhos de
  cada faixa ficam intactos (`heap::self_test`). `RUSTOS_CPU=a64fx` (TCG, bloco de
  256 bytes) cobre blocos maiores que 64, e com isso as pontas `head` e `tail`. O laço de
  `STP` de `DZP` ligado foi rodado à mão no HVF forçando o recuo, com o mesmo autoteste
  verde; nenhum modelo da QEMU liga `DZP`, e o `make smoke` não o cobre
  ([DEBT-0060](../tech-debt/0060-recuo-do-zerar-sem-cobertura-no-smoke.md)).

## Página grande

O mapa direto usa **2 MiB onde o endereço virtual e o físico alinham, e 4 KiB nas pontas**
(`runs` em `core/mm/src/ranges.rs`, que o `map_range` de `kernel/src/vm.rs` segue; `coalesce` junta as regiões vizinhas ou sobrepostas antes). Imagem e framebuffer usam a mesma função; a imagem,
que tem seções de 4 KiB por permissão, não chega a usar 2 MiB.

| QEMU, 256 MiB, `dev` | x86_64 | aarch64 |
| --- | --- | --- |
| Só 4 KiB | 64 939 páginas, 548 KiB de tabelas | 64 107 páginas, 536 KiB de tabelas |
| 2 MiB onde dá | 122 de 2M e 2 475 de 4K, 60 KiB de tabelas | 123 de 2M e 1 131 de 4K, 44 a 48 KiB |

Os 4 KiB que sobram são as pontas de região e o framebuffer (1 000 páginas no x86_64).
O motivo é o TLB: com 2 MiB, 256 MiB de mapa direto são 128 entradas em vez de 65 536.
**Não há número de TLB**: a QEMU não o modela, e a medida fica para metal
(inferência: o ganho vem da razão 512:1 de entradas, não de ciclos medidos).

O preço é que o mapa direto **não muda permissão por frame**. Ele é todo leitura e
escrita sem execução, e nada o precisa dividir. Quem precisar (página de guarda no mapa
direto, por exemplo) parte a página grande então. **1 GiB não entra:** exigiria testar
`pdpe1gb` no x86_64 e, com 256 MiB, as tabelas já são 60 KiB; sem número para justificar.

## Pilha própria

`Memory::map_stack(vm.space_mut(), 16)`: 16 páginas (64 KiB) e uma de guarda abaixo, na
janela de pilhas. 64 KiB é o mínimo que o Limine garante e com que o kernel rodou até
aqui. O pico medido do shell (`ps`, linha `stacks:` do boot) é de 12,9 a 16,0 KiB no boot e de 13,7 a 17,5 KiB depois do `bench`
([SDD 15](15_tarefas_e_preempcao.md#pico-de-pilha)): 3,7x de folga, e a pilha segue em 64 KiB até a 4a remedir. A troca é um `asm!` por ISA
(`call_on_stack(top, entry, arg)`): grava `sp`, zera o frame pointer e chama `entry(arg)`,
que nunca volta. Os dados passam por `Box::into_raw`, não pela pilha antiga.

Cada tarefa do kernel tem a sua, do mesmo `map_stack` (4 páginas, 16 KiB, e uma de guarda; o tamanho vem da medida de pico), e
reusa a do slot que ocupa ([SDD 15](15_tarefas_e_preempcao.md)). A janela de pilhas é só de
avanço, então encerrar uma tarefa não devolve endereço. No aarch64 o `call_on_stack`
também passa o kernel para EL1h.

## Memória do bootloader devolvida

`Memory::reclaim_bootloader` só roda depois de três trocas: tabelas (passo 4), pilha
(passo 5) e GDT (x86_64, `install_traps`; o aarch64 instala `VBAR_EL1`). Faz
`release_range` de cada região `BootloaderReclaimable`, passa o tipo delas a `Usable` no
`BootRecord` e refaz o `MemorySummary`; por isso `fetch` e `mem` contam certo.
`summarize` agora estende o `managed_span` do buddy às regiões recuperáveis, senão o
buddy não as cobriria. O boot imprime `[rustos] reclaimed: N KiB from the bootloader`.

| `dev`, QEMU 256 MiB | x86_64 | aarch64 |
| --- | --- | --- |
| Frames livres, antes da fase 2b | 52 321 (204,4 MiB) | 54 991 (214,8 MiB) |
| Frames livres, depois | 63 811 (249,3 MiB) | 63 237 (247,0 MiB) |
| Devolvido | 46 432 KiB | 33 424 KiB |

Quase tudo é memória de boot do firmware e do Limine (a cópia do ELF dele, as tabelas, a
pilha). O que o Linux devolve não foi medido aqui.

## W^X verificado

- **No boot.** `Vm::check_wx` percorre as tabelas novas (`AddressSpace::walk`) e passa
  cada folha ao `rustos_mm::WxChecker` (`core/mm/src/wx.rs`, testado no host). Recusa:
  página gravável e executável em qualquer lugar; seção da imagem com permissão diferente
  da esperada ou sem mapeamento; mapeamento abaixo de `0xffff_0000_0000_0000`;
  qualquer outro mapeamento (o mapa direto) que aponte para a memória física da imagem.
  Imprime `[rustos] w^x: ok (N pages)`, com N em páginas de 4 KiB, ou `FAILED`. A janela
  do initramfs entra na conta, com as folhas só de leitura.
  O `make smoke` exige a linha `ok`.
- **RELRO.** Com PIE, vtables e tabelas de ponteiros (`interrupts::HANDLERS`, a tabela de
  comandos do shell) saem em `.data.rel.ro`, que o `*(.data .data.*)` engolia: ficavam
  graváveis depois do boot. O linker script põe `.data.rel.ro` dentro de `.rodata`, entre
  `__relro_start` e `__relro_end`; o Limine relocou antes de entregar o controle. O
  `check_wx` confere a faixa como seção só leitura e que uma tabela de ponteiros plantada
  pelo kernel (`RELRO_CANARY`) caiu nela; sem isso, `Violation::RelroMissing`. Medido no
  `dev`: `HANDLERS` foi de `.data` para a faixa RELRO; sobraram em `.data` só 6
  relocações no x86_64 e 1 no aarch64, todas de `static` mutável (`SCREEN`, `KEYMAP`, `HEAP`...).
- **Executável só em `.text`.** O `WxChecker` recusa página executável que não caiba
  inteira numa seção executável (`Violation::ExecuteOutsideText`).
- **No shell.** `fault text` lê o primeiro byte de `probe_write_text` (em `.text`) e o
  regrava: com a proteção, é page fault de proteção e não volta; sem ela, não altera
  nada e o shell avisa `no fault was raised`. Saída medida (x86_64):
  `page fault at 0xffffffff80007ce0 (write, kernel, protection)`; no aarch64, o mesmo
  texto com `ESR 0x9600004f` (permission fault, nível 3, escrita).

O autoteste lê as tabelas que o kernel escreveu; quem prova que a CPU as respeita é o
`fault text`. Os dois juntos fecham a regra do [guidelines 09](../guidelines/09_seguranca.md).

## KASLR

O kernel é um executável independente de posição (PIE), e o Limine aplica a relocação e
sorteia a base (PROTOCOL.md, *Memory Layout at Entry*: "free to relocate it ... potentially
performing slide randomisation").

O que o ELF precisa (descoberto ao testar; o PROTOCOL.md não diz):

- **`PT_DYNAMIC` no ELF.** Sem ele, o Limine para com `ET_DYN, but PT_DYNAMIC segment
  missing`. O linker script tem `dynamic PT_DYNAMIC` e põe `.dynamic` e `.got` no
  segmento de `.requests`.
- **Só relocação relativa** (`R_X86_64_RELATIVE`, `R_AARCH64_RELATIVE`): 888 no `dev` do
  x86_64 e 648 no do aarch64.
- **`kaslr: yes` no `limine.conf`** (`xtask/src/image.rs`). Sem a linha, a base fica em
  `0xffffffff80000000` mesmo com o ELF relocável.
- **`-C relocation-model=pie`** nos dois alvos (`.cargo/config.toml`) e `-pie` no
  link (`kernel/build.rs`). No aarch64 a `core` pré-compilada é estática; o link só passa
  com `-z notext`, que deixa relocações em `.rodata` (o Limine as aplica antes de o
  kernel mapear `.rodata` como leitura) e marca `DT_FLAGS: TEXTREL`
  ([DEBT-0024](../tech-debt/0024-aarch64-pie-com-textrel.md)).

Resultado, 5 boots por ISA: a base muda a cada boot, em múltiplos de 4 KiB dentro dos 2
GiB do topo (x86_64: `0xffffffffd42e8000`, `0xffffffffda099000`, `0xffffffff9aee3000`; aarch64:
`0xffffffffec001000`, `0xffffffffce237000`, `0xffffffffc9e68000`), e o deslocamento do
mapa direto também (`0xffff831f80000000`, `0xffff9c6cc0000000`...). A fonte da aleatoriedade
é a do Limine, não o gerador do kernel ([DEBT-0019](../tech-debt/0019-aleatorio-sem-fonte-de-hardware-fraco.md)
não se aplica). O boot não imprime a base: só `kaslr: on` ([SDD 09](09_seguranca.md#endereços-do-kernel-no-log));
`RUSTOS_FEATURES=debug-addrs` a mostra.

**Por causa do sorteio do mapa direto, as janelas de dispositivos, de pilhas e de módulos não têm
endereço fixo.** Com o endereço fixo da fase 1 (entradas 510 e 509), o mapa direto sorteado
poderia cair sobre elas (inferência: 1 TiB de 128 TiB, cerca de 1 boot em 100; não medido). `free_top_level_slot` escolhe a entrada
depois de conhecer o mapa direto e a imagem.

Custo medido (`make size`, soma das seções carregadas; mesma árvore, só o PIE muda):

| | x86_64 `release` | x86_64 `release-small` | aarch64 `release` | aarch64 `release-small` |
| --- | --- | --- | --- | --- |
| Estático | 179 608 | 113 743 | 149 776 | 95 180 |
| PIE | 182 897 (+3 289, 1,8%) | 119 324 (+5 581, 4,9%) | 155 137 (+5 361, 3,6%) | 101 445 (+6 265, 6,6%) |

O ganho está em `.rela.dyn` (24 bytes por relocação) e `.got`, e em vtables e tabelas
de ponteiros que saem de `.rodata` para `.data.rel.ro` (só leitura, ver *RELRO*). O tempo do Limine
(`bootloader N us`) ficou no ruído: aarch64 sob HVF, 43 a 52 ms com e sem PIE.
Decisão: **KASLR ligado**, porque o custo é de 3 a 6 KiB e a base muda mesmo.

## Decisões

| Decisão | Por quê | Fonte |
| --- | --- | --- |
| Tabelas novas antes do console | Console, tela e interrupções já mapeiam nelas | pedido da tarefa |
| Mapa direto sem a imagem, sem dispositivo, sem ACPI | Alias gravável de `.text` anula W^X; dispositivo passa pela janela | PROTOCOL.md (o Limine mapeia tudo isso; o kernel escolhe não repetir) |
| 2 MiB no mapa direto | 488 KiB e 512x menos entradas de TLB | tabela acima |
| Janelas depois do mapa direto | O Limine sorteia o deslocamento | PROTOCOL.md, *Memory Layout at Entry* |
| 64 KiB de pilha | Mínimo do Limine, com que o kernel já rodava | PROTOCOL.md, *Machine State at Entry* |
| KASLR por PIE e Limine | Custo de 3 a 6 KiB, base muda a cada boot | medida acima |
| Uma caminhada genérica em `core/mm`, sobre `PageFormat`, `TableMemory` e `MmuControl` | A caminhada era a mesma nas duas ISAs; a dívida da cópia por ISA está paga | Caso 1 do `AGENTS.md` (porta nova no `hal`, sem crate novo) |
| Dois TLB flush na troca do aarch64 | Evita duas traduções do mesmo endereço (bloco de tamanho diferente) vivas ao mesmo tempo | Arm ARM, *TLB conflict abort* (não lido nesta rodada) |

## O que a QEMU não prova

Write-combining e atributo de framebuffer não têm efeito observável na QEMU: o que se
confere é a codificação (testes no host) e o PAT lido do registrador. Vale rodar em
metal ([DEBT-0014](../tech-debt/0014-x86-sem-salvaguardas-de-metal.md)).
