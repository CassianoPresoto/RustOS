# SDD 18 — M2: DOOM nativo por software

> **O que este capítulo responde:** o que o M2 entrega, em que ordem do roteiro ele entra,
> como o DOOM (doomgeneric), o WAD e a ferramenta de build entram no repositório sem
> violar o `AGENTS.md`, e quais contratos novos (framebuffer, entrada, libc) o programa
> enxerga.

**Estado: desenho aprovado (#46, 2026-11-24); a fatia 1a (contrato de tela) está no código.** O dono
aprovou as sete decisões com as recomendações ([Decisões](#decisões)); o que abaixo ainda diz **Proposta**
é o desenho aprovado, que passa a descritivo à medida que cada fatia do M2 entra. A fatia 1a está descrita em
[Como entrou](#como-entrou-fatia-1a).
O M2 é o marco 1 dos jogos da [escada do SDD 08](08_roadmap.md#jogos-a-escada), antecipado
para logo depois da fase 4, sobre a ABI nativa do [SDD 17](17_processos_isolamento_e_syscalls.md).

Marcas usadas, como no [SDD 17](17_processos_isolamento_e_syscalls.md#como-ler-as-marcas):
**Verificado** (lido na fonte, com o link), **NÃO VERIFICADO** (não lido; não é premissa),
**Proposta** (desenho que o dono ainda aprova).

## O M2 e o que ele fecha

O M2 fecha quando os três itens abaixo valem no x86_64 (o terceiro é o que o dono
joga; o primeiro e o segundo são a medida):

| # | Critério | Como se confere |
| --- | --- | --- |
| 1 | `doom -timedemo demo1` roda e imprime o resultado do timedemo (tempo em tics de jogo e em tempo real, e os quadros por segundo) | `make smoke` no x86_64, com a linha lida da serial |
| 2 | O mesmo build do doomgeneric (mesmo WAD, mesmo `-timedemo demo1`) roda no Linux nas mesmas condições: QEMU, mesma CPU e mesma memória | medida pareada, [seção de medida](#medida-contra-linux-e-windows) |
| 3 | Dá para jogar no x86_64 com teclado PS/2 | teste manual do dono, em QEMU |

**Proposta (decisão 1):** no aarch64, o `timedemo` não precisa de entrada, então pode fechar
o item 1 no aarch64 já no M2. Jogar no aarch64 fica para quando houver PCI e virtio-input
(fase 6). A alternativa é fechar o M2 só no x86_64, como a escada previa.

### Por que na fase 4 e não na 5

A escada do [SDD 08](08_roadmap.md#jogos-a-escada) punha o marco 1 na fase 5, junto da ABI Linux.
O DOOM não precisa dela: precisa de processo, ELF, handles, VMO, canal e de `clock_get_monotonic`,
que a fase 4 já entrega. Puxar o marco para cá deixa a fase 5 com a ABI Linux e o BusyBox, e
mede o desempenho antes de a camada de compatibilidade existir.

## Dependências que o M2 já tem

| Pré-requisito | Onde está | Marca |
| --- | --- | --- |
| Processo de usuário, ELF64 estático, `init` e programas do initramfs | [SDD 17](17_processos_isolamento_e_syscalls.md#executável-e-carregador), M1 | **Já é assim** (M1 feito) |
| `clock_get_monotonic` (ns desde o boot, `u64`) | [ABI nativa](17_processos_isolamento_e_syscalls.md#as-chamadas) | **Já é assim** na 4a; ressalva de TSC entre núcleos ([DEBT-0057](../tech-debt/0057-tsc-entre-nucleos-sem-verificacao.md)) |
| Canal, VMO, VMAR, `process_create`, `object_wait_one` | [SDD 17](17_processos_isolamento_e_syscalls.md#modelo-de-objetos-e-handles), fase 4c | **Já é assim** |
| Framebuffer linear de 32 bpp XRGB, desenhado pelo console | [SDD 04](04_boot.md) e [SDD 16](16_terminal.md) | **Já é assim** |
| Teclado PS/2 com press e release, no x86_64 | [SDD 11](11_teclado.md) (`drivers/i8042`, `Set1Decoder`) | **Já é assim** |
| Initramfs com o WAD e o binário do DOOM | `kind 4`, `core/cpio`, `xtask initramfs` | **Já é assim** para o initramfs; o WAD e o binário são a decisão 4 |

O que o M2 acrescenta: a libc mínima (decisão 7), o framebuffer e a entrada como contrato de
usuário (decisões 5 e 6), e o alvo de build do C (decisão 3).

## Código de terceiros: doomgeneric

**Caso 2 do `AGENTS.md`.** Fontes lidas nesta rodada, pela API do GitHub e por clone raso
(só para leitura, fora do repositório):

| Fato | Fonte | Marca |
| --- | --- | --- |
| Repositório `ozkl/doomgeneric`, licença `gpl-2.0` (GPL-2.0), último push em 2026-10-11 | `gh api repos/ozkl/doomgeneric` (campo `licenseInfo`); clone no commit `e257a095` | **Verificado** pela API; **o texto do arquivo `LICENSE` não foi lido** por mim, só o metadado |
| API de porte: `DG_Init`, `DG_DrawFrame`, `DG_SleepMs`, `DG_GetTicksMs`, `DG_GetKey(int* pressed, unsigned char* key)`; `DG_SetWindowTitle` é opcional | [`doomgeneric/doomgeneric.h`](https://github.com/ozkl/doomgeneric/blob/master/doomgeneric/doomgeneric.h) e README | **Verificado** |
| Entrada: `doomgeneric_Create(argc, argv)` e `doomgeneric_Tick()` em loop | `doomgeneric.c` e README | **Verificado** |
| `DOOMGENERIC_RESX` padrão 640, `DOOMGENERIC_RESY` padrão 400, por `#ifndef` (podem ser definidos no build) | `doomgeneric.h` | **Verificado** |
| Pixel de 32 bits por padrão (`pixel_t = uint32_t`); `CMAP256` troca para 8 bits; `DG_ScreenBuffer` é alocado com `malloc(RESX*RESY*4)` | `doomgeneric.h` e `doomgeneric.c` | **Verificado** |
| Som desligado por padrão (`FEATURE_SOUND` comentado em `doomfeatures.h`) | `doomfeatures.h` | **Verificado** |
| Backends prontos: `doomgeneric_xlib.c`, `doomgeneric_sdl.c`, `doomgeneric_win.c`, `doomgeneric_linuxvt.c` (este abre `/dev/fb0`) | listagem da pasta `doomgeneric/` | **Verificado** (existência; não compilei nenhum) |

**Decisão 2 — como o código entra.** Opções:

- **(a) baixar no build um commit fixo, com checksum, sem vendorizar.** O repositório guarda
  o hash do commit e o SHA-256 do tarball; o xtask baixa para `target/` e confere antes de
  compilar. O código de terceiros não fica no git.
- **(b) vendorizar** o diretório `doomgeneric/` no repositório, com um `README` do porte.

**Recomendação: (a).** Razões: o repositório fica MIT/Apache sem código GPL dentro; o build
fica reprodutível pelo hash; a atualização é uma linha de diff. O custo é depender de rede
no primeiro build (e de cache depois). Sem rede, o M2 fica desligado (ver decisão 3).

**Efeito da licença (GPL-2.0).** O programa `doom` (doomgeneric mais o porte e a libc do
RustOS que ele liga estaticamente) é um binário derivado de GPL-2.0, então **ele** é GPL-2.0 e
fica fora do repositório. O kernel, o `user/rt` e os demais programas não são afetados, porque
são programas separados que falam com o kernel por syscall (a ABI nativa), e não ligam nem
incorporam o código do DOOM. O `doom` distribuído junto com a imagem precisa de aviso de
licença e do código-fonte correspondente. **NÃO VERIFICADO:** a leitura jurídica desse efeito
(o que conta como "derivado" para a GPL-2.0 com uma libc própria) é do dono, não do desenho.

**Proposta de porte, dentro de `user/`.** Um crate `user/doom` com o C do doomgeneric (fora
do repositório, pelo passo de build) e um arquivo `doomgeneric_rustos.c` no porte: implementa
as cinco funções `DG_*` sobre o contrato dos [contratos de tela e entrada](#contrato-de-tela-framebuffer-caso-6)
e sobre `clock_get_monotonic` (`DG_GetTicksMs` = ns / 1 000 000). `DG_SleepMs` usa `nop` com
espera de `object_wait_one` com prazo, ou um laço de `clock_get_monotonic` (**Proposta**; a
escolha entra no PR da fatia).

## Ferramenta de build: clang

**Caso 2 do `AGENTS.md`** (binário externo novo). Flags conferidas na fonte do clang, no
arquivo de opções do upstream (`clang/include/clang/Options/Options.td`, lido pela API do
GitHub no `master`):

| Flag | O que faz | Marca |
| --- | --- | --- |
| `-ffreestanding` | "Assert that the compilation takes place in a freestanding environment" | **Verificado** (Options.td) |
| `-nostdlib` | Não liga com as bibliotecas padrão do sistema | **Verificado** que a flag existe no Options.td (grupo `Link_Group`); o efeito exato no link **NÃO VERIFICADO** nesta rodada |
| `-nostdinc` | Desliga os diretórios de include do sistema e os embutidos | **Verificado** (Options.td); ler a página do clang para o efeito completo: NÃO VERIFICADO |
| `--target=<triple>` | "Generate code for the given target" | **Verificado** (Options.td, `def target`); e no `clang --help` do Apple clang 21 local (secundária) |

**Proposta de chamada pelo xtask** (para cada alvo, `x86_64-unknown-none` e `aarch64-unknown-none`,
com o triple do clang `x86_64-unknown-none-elf` e `aarch64-unknown-none-elf`: **NÃO VERIFICADO**
que o triple de cada um seja aceito pelo clang de Homebrew e pelo do LLVM de Linux):

```
clang --target=<triple> -ffreestanding -nostdlib -nostdinc -O2 -c <porte e fontes do doomgeneric>
```

O link não sai do clang: a proposta é ligar com o `rust-lld` que já vem com o `rustc`
(**NÃO VERIFICADO**: a presença do `rust-lld` no toolchain do dono e a versão), com o mesmo
linker script de `user/link/*.ld` que os programas Rust já usam. Se for preciso um linker
externo, vira decisão nova.

**Decisão 3 — o que acontece sem clang.** O M2 vira **opcional no build e no CI**: `make build`
e `make pr` pulam o `doom` com aviso quando não acham o clang, e o smoke do M2 só roda com o
clang presente (`make smoke-doom`, novo alvo). Sem o clang, o resto do repositório continua
verde. **Opção:** o clang vira obrigatório a partir do M2 (o CI falha sem ele). Recomendo o
opcional até o M2 fechar, e o obrigatório quando o timedemo for medição publicada.

## WAD: o que o timedemo usa

O `-timedemo` do doomgeneric chama `G_TimeDemo(demolumpname)` (`d_main.c`, linhas 1479 e 1821;
o nome é `demo1`, sem extensão, e `.lmp` também é aceito).

| Fato | Fonte | Marca |
| --- | --- | --- |
| O resultado do timedemo sai por `I_Error("timed %i gametics in %i realtics (%f fps)")` | `g_game.c`, linha 2265 | **Verificado** |
| `I_Error` escreve a mensagem em `stderr` (`vfprintf`), roda as funções de saída e termina com `exit(-1)` (caminhos de erro de `i_system.c`) | `i_system.c`, `I_Error` | **Verificado** (o caminho exato de saída depende do `ORIGCODE`; lido, não testado) |
| O fps é `gametic * TICRATE / realtics`, com `TICRATE` de 35 | `g_game.c`, linhas 2250–2265 | **Verificado** |
| Freedoom: `DEMO1` existe nas duas fases, com `demo1_p1.lmp` (Fase 1, E1M6, singleplayer, por mc776) e `demo1_p2.lmp` (Fase 2, MAP03, singleplayer, por mc776) | `lumps/README.DEMO` e `buildcfg.txt`, linhas 186–200, no commit `e4ec24d` (2026-11-23) | **Verificado** na fonte; **o WAD construído não foi baixado**: que o `freedoom1.wad` do release 0.13.0 contenha o lump `DEMO1` é **NÃO VERIFICADO** |
| Freedoom: licença do conjunto principal (`COPYING.adoc`), BSD-3-Clause na forma: redistribuição com ou sem modificação, com a nota de direito autoral | `COPYING.adoc` (lido) | **Verificado**; a licença de cada música (`CREDITS-MUSIC`) **NÃO VERIFICADA** |
| Freedoom: release `v0.13.0` (2024-01-29) com `freedoom-0.13.0.zip` e `freedoom-0.13.0-CHECKSUM` | `gh release view` | **Verificado** (nome dos assets); o conteúdo do zip e o checksum **NÃO VERIFICADOS** (não baixados) |

**Decisão 4 — de onde vem o WAD.** Opções:

- **(a) Freedoom**, baixado no build com checksum (como o doomgeneric), para CI e para o timedemo.
- **(b) `doom1.wad` do usuário**, colocado numa pasta local e nunca no repositório (é o WAD
  comercial; a licença não permite redistribuição).
- **(c)** os dois: Freedoom no CI, `doom1.wad` local como opcional.

**Recomendação: (c).** O CI e o timedemo usam Freedoom, para o resultado ser reproduzível e
sem ativo proprietário; `doom1.wad` fica como opcional local, e o timedemo com ele é a
comparação com o que o jogador conhece. **Ressalva:** os demos do Freedoom não são os do
`doom1.wad`. `demo1` do Freedoom grava E1M6 (Fase 1) e não E1M1; o número de quadros e o
tempo mudam de WAD para WAD, então **o número do M2 só se compara com o Linux usando o mesmo
WAD**. O número publicado usa a **Fase 1 do Freedoom** (`freedoom1.wad`, `demo1` em E1M6): o
formato do DOOM 1, o mesmo do `doom1.wad` (escolha feita na aprovação do #46, com o aval do dono
às recomendações). **NÃO VERIFICADO:** se o `demo1` do Freedoom roda sem falha de
sincronia no doomgeneric (não rodei).

## Contrato de tela (framebuffer), caso 6

**Proposta, decisão 5 (`kind` 5 Framebuffer).** Hoje o framebuffer é do kernel: o console o
desenha (SDD 16), e o usuário não o enxerga. O M2 precisa que o DOOM escreva pixels direto.

**Layout proposto** (`#[repr(C)]`, 16 bytes, na mensagem de formato que acompanha o `kind` 5):

| Offset | Campo | Tipo | Valor |
| --- | --- | --- | --- |
| 0 | `width` | `u32` | largura em pixels |
| 4 | `height` | `u32` | altura em pixels |
| 8 | `stride` | `u32` | bytes por linha (pode ser maior que `width * 4`) |
| 12 | `format` | `u32` | `1` = XRGB8888 (32 bpp, 8 bits por canal, nesta ordem; a única aceita pelo boot, [SDD 04](04_boot.md)) |

**Como chega.** O `kind` 5 é um VMO físico do framebuffer (`READ`, `WRITE`, `MAP`, `DUPLICATE`,
`TRANSFER`, sem `EXECUTE`), com a política de cache de vídeo (write-combining, como o
`PageFlags::FRAMEBUFFER` do [SDD 13](13_memoria_virtual.md)) no `vmar_map`. O programa mapeia
com `vmar_map` e escreve na memória. A mensagem de formato vai **junto** da mensagem de boot,
como um bloco fixo depois dos `kind`s; isto muda o layout da mensagem de boot de versão 1 para
versão 2 (**Proposta**, e a mudança é contrato, pelo caso 6). **Alternativa:** uma syscall
`framebuffer_info` (número novo) em vez de bloco na mensagem. Decisão do dono se a versão 2 for
aceita.

**Opção de mapeamento, decisão 5a (o que a syscall de tela faz).**

- **(a) VMO direto, sem "present".** O programa escreve no próprio mapeamento e o hardware
  mostra. **Recomendo (a)**: o framebuffer do Limine é memória de vídeo mapeada; não há cópia
  nem sincronização a fazer, e o custo por quadro é só a escrita. Com (a) o M2 mede o custo real
  do DOOM (a escrita WC é parte da medida), sem uma cópia extra que o Linux não faria.
- **(b) syscall `framebuffer_present` com cópia** de um buffer do programa para o quadro. Custa
  uma cópia de `width*height*4` bytes por quadro (640x400 são 1 MiB), e entra no número como
  custo do RustOS que o Linux não paga na mesma medida. Só se (a) for inviável.

**Regra de posse (proposta).** Enquanto um processo segura um handle do `kind` 5, o console do
kernel **não desenha** no framebuffer; a serial segue. Quando o último handle do VMO fecha (ou o
processo morre, o que fecha os handles dele), o console volta e redesenha a tela inteira.
**Ponto aberto:** um mapeamento do VMO sobrevive ao fechamento do handle, porque o VMO fica vivo
enquanto houver mapeamento. Proposta: a posse é pelo **mapeamento** (o console volta quando o
último mapeamento do framebuffer sai, na morte do processo ou no `vmar_unmap`), e o handle só
aponta o VMO. **Decisão 5b** para o dono: posse pelo handle (o texto do pedido) ou pelo
mapeamento (esta proposta, que não deixa o console desenhar sobre memória que o processo ainda
mapeia).

**Custo que o SDD 17 já prevê:** "política de cache no `vmar_map`" está na [tabela de
decisões do agente](17_processos_isolamento_e_syscalls.md#decisões-do-agente) como
caminho aberto para o framebuffer; este capítulo é quem a usa.

### Como entrou (fatia 1a)

O contrato de tela está no código, com o desenho aprovado e as lacunas que ele deixava decididas aqui:

- **VMO de framebuffer** (`kernel/src/vmo.rs`, `Kind::Framebuffer`): físico, sobre a memória da tela. Não cobra
  quota e não devolve frames ao morrer (como o initramfs). O mapeamento usa `PageFlags::FRAMEBUFFER`
  (write-combining) pelo `Vmo::cache`, na falta de página da região (`FaultPlan::MapObject`). Direitos:
  `READ`, `WRITE`, `MAP`, `DUPLICATE`, `TRANSFER`, sem `EXECUTE` ([SDD 17](17_processos_isolamento_e_syscalls.md#direitos-iniciais-decisão-3)).
- **Quem recebe o `kind` 5**: o programa que o `run` do shell lança (`kernel/src/programs.rs`, opção
  `framebuffer`) e o `fb` do autoteste. O `init` e os filhos que ele cria não recebem.
- **Mensagem de boot v2** (`core/hal/src/abi.rs`): o bloco de formato de 16 bytes depois dos `kind`s, só com
  `kind` 5. O kernel escreve a v2 sempre; o `user/rt` lê a v1 e a v2 e expõe `Boot::framebuffer`.
- **Posse** ([SDD 16](16_terminal.md#a-posse-da-tela-framebuffer-como-vmo)): a contagem é de mapeamentos, como
  recomendado (decisão 5b). Só o handle, sem mapear, não dá a tela a ninguém.
- **Teste `fb`** (`user/tests/fb`): lê o formato, mapeia, pinta 64 linhas, avisa o kernel (`mapped`), espera
  `go`, confere que o padrão segue e sai. O autoteste confere que o console ficou quieto (`skipped` subiu, o
  pixel (0, 0) segue com o padrão) e que redesenhou depois (o pixel (0, 0) volta à barra).

**Decisões que fecham as lacunas do desenho:**

- **5c, formato.** O bloco `format` só descreve XRGB8888: canais de 8 bits em `red` 16, `green` 8, `blue` 0. Outra
  ordem de canais (que o boot aceita, [SDD 04](04_boot.md)) fica **sem** `kind` 5, e o console desenha normal.
  Dívida [DEBT-0159](../tech-debt/0159-framebuffer-com-outra-ordem-de-canais-sem-vmo.md).
- **5d, `stride` em bytes.** É o `pitch` do bootloader. O VMO tem `ceil(stride * height / 4096)` páginas; a ABI
  não diz o tamanho do VMO ([DEBT-0140](../tech-debt/0140-abi-sem-tamanho-de-vmo.md)), e o formato diz.
- **5e, pânico.** `print_panic` desenha mesmo com a tela mapeada por um programa. O pânico é a última saída, e um
  programa travado não deve esconder a causa.
- **5f, redesenho no laço do shell.** A morte do processo levanta a marca de redesenho e acorda o laço do shell
  com `notify_input` (que roda com a IRQ mascarada, como na IRQ de teclado). O predicado de espera do laço
  inclui a marca, então a tela volta sem tecla. Quem redesenha é o `flush` desse laço, que roda com a IRQ
  mascarada (`read_input_blocking`), como qualquer `flush` ([DEBT-0051](../tech-debt/0051-redesenho-da-tela-com-irq-mascarada.md)).
- **5g, espera pela renderização em curso.** Cada mapeamento de framebuffer (cada `vmar_map` que dá certo) espera o
  lock da tela (`console::settle`) depois de soltar o lock do processo, para que nenhuma renderização que começou
  antes escreva por cima do padrão depois que o endereço volta ao programa.

**Medido (fatia 1a):** `make smoke ARCH=all`, `smoke-fast` nas duas ISAs e a imagem enxuta passam; o autoteste
`fb` roda nas duas ISAs (x86_64 1280x800, stride 5120; aarch64 ramfb 1024x768, stride 4096). O tamanho do kernel
`release-small` (x86_64) sobe de 278 199 para 280 183 bytes carregados (+1 984, +0,7%); a medida do custo do
framebuffer por quadro fica para a fatia do DOOM, com a mesma medida antes e depois.

## Contrato de entrada, caso 6

**Proposta, decisão 6 (`kind` 6 Input).** Um canal em que o kernel escreve um evento por tecla
(o programa lê com `channel_read`, `object_wait_one` para `READABLE`).

**Layout proposto** (`#[repr(C)]`, 16 bytes, uma mensagem por evento, sem handles):

| Offset | Campo | Tipo | Valor |
| --- | --- | --- | --- |
| 0 | `code` | `u16` | código da tecla (ver abaixo) |
| 2 | `pressed` | `u8` | `1` press, `0` release |
| 3 | `reserved` | `u8` | 0 |
| 4 | `reserved2` | `u32` | 0 |
| 8 | `time_ns` | `u64` | `clock_get_monotonic` no momento do evento |

**Código: decisão 6a.** Opções:

- **(a) HID Usage do `hal::KeyCode`** (página 0x07, o que o decodificador já produz; [SDD 11](11_teclado.md)).
  O `Set1Decoder` já converte o conjunto 1 em HID byte a byte, então este código sai do mapa que
  existe, sem conversão nova. **Recomendo (a).**
- **(b) scancode set 1 cru**, o byte que o i8042 entrega. Exige o canal guardar o byte antes do
  decodificador, e o decodificador deixa de ser o dono do mapa; o programa de usuário teria de
  conhecer o conjunto 1 (com `E0`/`E1`). Mais código do lado do usuário, sem ganho.
- **(c) código próprio** (um número por tecla, sem relação com HID nem set 1): mesma tabela
  nova, sem razão além de independência do mapa; não recomendo.

**Release no aarch64: decisão 6b.** O i8042 dá press e release (SDD 11). No aarch64 não há teclado
(a `virt` não tem PS/2; o teclado USB é da fase 8, e o virtio-input da fase 6), e a serial não
dá release. Opções: (a) **nada**: o canal de entrada não existe no aarch64 no M2 (o timedemo não
precisa); (b) **release sintético depois de N ms** do press, para a serial, com N escolhido pelo
dono. **Recomendo (a)**: o jogo no aarch64 não é critério do M2, e um release inventado muda o
comportamento do jogo sem o usuário saber.

**Canal:** 16 mensagens na fila (o limite da [ABI](17_processos_isolamento_e_syscalls.md#contrato-numérico)),
e cada evento cabe em 16 bytes. Se a fila encher, o kernel descarta o evento mais antigo ou
guarda o mais novo? **Proposta:** descarta o **mais antigo** e conta; a contagem sai em
`debug_write` de diagnóstico (não é contrato). Decisão do PR da fatia, não deste.

## Libc mínima

**Decisão 7 — caso 3 (regra do guidelines).** O doomgeneric precisa de libc. Medi a lista
pelo código-fonte (`grep` sobre os `.c` e `.h` do clone; contagem aproximada, incluindo os
backends que não entram no DOOM bare-metal):

| Família | Funções que o núcleo do DOOM usa (no código comum) | Observação |
| --- | --- | --- |
| Memória | `malloc`, `calloc`, `realloc`, `free`, `memcpy`, `memmove`, `memset`, `memcmp` | Heap do `user/rt` |
| Strings | `strlen`, `strcmp`, `strncmp`, `strncpy`, `strchr`, `strrchr`, `strstr`, `strdup`, `strerror` | |
| Ctype | `isspace`, `isprint`, `isalpha`, `toupper`, `tolower` | |
| Conversão | `atoi`, `atof`, `sscanf` (config e argumentos) | `sscanf` é o mais chato |
| Saída | `printf`, `fprintf(stderr)`, `vsnprintf`, `puts`, `putchar`, `fflush` | **`%f` é necessário**: o resultado do timedemo usa `%f` (`g_game.c`) |
| Arquivo (leitura) | `fopen`, `fread`, `fseek`, `ftell`, `fclose`, `feof` | Para o WAD; o initramfs é a origem |
| Arquivo (escrita) | `fwrite`, `remove`, `rename` | Usados por savegame e config; **sem escrita no M2**: retornam erro |
| Processo | `exit`, `abort`, `atexit`, `getenv` (devolve `NULL`), `assert` | `atexit` roda as funções de `I_Error` |
| Matemática | `sin`, `tan`, `fabs` | Tabelas de `r_main.c`; sem `libm` externo |

Não usa: `open`/`read`/`close` (só o `doomgeneric_linuxvt.c`, que fica fora), `time`, `rand`,
`setjmp`. Confirmado: `time(` aparece só num comentário.

**Opções:**

- **(a) crate novo `user/libc`**, em Rust, exportando a ABI C com `#[unsafe(no_mangle)] extern "C"`.
  Precisa de `unsafe` **fora de `user/rt`**, então é exceção à regra 11. **Recomendo (a)**: fica
  um lugar só para a ABI C, a regra 11 ganha uma exceção explícita, e o `user/rt` continua mínimo.
- **(b)** pôr a libc dentro de `user/rt`, sem mudar a regra. Inchaço: todo programa do sistema
  paga a libc, e a meta de memória (SDD 01) conta. Rejeitada.
- **(c)** libc C de terceiros, **musl**: caso 2 de novo, e é pesada (o próprio SDD 08 cita a fase 5
  para ela). Rejeitada para o M2.

**Diff proposto da regra 11** em [`guidelines/03`](../guidelines/03_camadas_e_dependencias.md)
(só no PR de decisão; aqui o diff está no próprio PR do Draft):

```
11. ... `unsafe` só no `user/rt`, mínimo e com `// SAFETY:` ...
    **Exceção (decisão 7a do SDD 18):** `user/libc` pode ter `unsafe` só nas funções
    `extern "C"` que a ABI C exige (ponteiros de C), com `// SAFETY:`; o crate tem
    `#![deny(unsafe_code)]` e `#[allow(unsafe_code)]` pontual. Nada depende de `user/libc`
    além de `user/doom` e de programas C ligados por `user/`.
```

`user/libc` depende de `core/hal` (módulo `abi`) e de `arch/<isa>-user`, como o `user/rt`; não
depende do `kernel/`. `make layers-check` precisa aprender a exceção (teste vermelho antes do verde,
como na 4a).

## Medida contra Linux e Windows

**O que compara.** O mesmo build do doomgeneric, o mesmo WAD, `-timedemo demo1`, o mesmo número
de CPUs (`-smp`) e a mesma memória. O número é o **quadros por segundo** que o timedemo imprime
(`fps` de `g_game.c`), mais o tempo em tempo real.

**Linux.** O mesmo doomgeneric com um backend do Linux: `doomgeneric_sdl.c` (depende de SDL2, que
é binário externo) ou `doomgeneric_linuxvt.c` (framebuffer `/dev/fb0`, sem SDL, mas precisa de root
e de framebuffer). Para a medida pareada, o backend é o que dá para rodar em QEMU sem janela: o
linuxvt, em um guest Linux com `fbdev` (**NÃO VERIFICADO**: que o guest da linha de base do
SDD 08, Alpine virt, dê `/dev/fb0` com o `vesafb`/`efifb` da QEMU; não testei). O guest Linux é só
instrumento de medida, como na linha de base de memória do [SDD 08](08_roadmap.md#memória-ociosa-o-alvo-vem-de-uma-medida).

**Windows.** `doomgeneric_win.c` existe no repositório (**Verificado**, existência). **Não é
comparação nas mesmas condições**: o Windows roda nativo no host, não na QEMU. A comparação com
Windows só é honesta se o Windows também rodar na mesma QEMU (uma VM com a mesma `-smp` e RAM), o
que é **NÃO VERIFICADO** quanto a viabilidade (licença e imagem do Windows fora do escopo do
repositório). Proposta: publicar o número do Windows só com a ressalva "nativo, outro host", ou
não publicar até existir VM.

**Diferenças de plataforma que mudam o número** (a serem registradas junto da medida):
TCG da QEMU no CI contra HVF no macOS (o SDD 07 já trata disso para o kernel); as escritas do
framebuffer em WC; o custo de `clock_get_monotonic` (TSC entre núcleos, [DEBT-0057](../tech-debt/0057-tsc-entre-nucleos-sem-verificacao.md)).

**Não verificado nesta rodada:** se o timedemo do doomgeneric roda sem espera de quadro (`D_DoomLoop`
com `DG_SleepMs`); se o `I_GetTime` do porte usa `DG_GetTicksMs` com a precisão esperada. Os dois são
lidos no código, não rodados.

## Roteiro e o que muda no SDD 08

O marco 1 sai da fase 5 e entra depois da fase 4 (M2). A fase 5 continua com a ABI Linux, o BusyBox
e a memória ociosa; o critério de DOOM sai dela. O [SDD 08](08_roadmap.md) muda:

- a linha da fase 5: o critério "DOOM por software roda no x86_64 (marco 1 dos jogos; no aarch64
  fecha na fase 6)" passa a apontar para o M2;
- a escada: o marco 1 sai da fase 5 para o M2 (depois da 4); o marco 2 (tempo) fica na fase 4 para o
  `clock_get_monotonic`, e o áudio continua na 6;
- as linhas de "Decisões futuras" que este Draft responde: DOOM e WAD, e a interface do framebuffer e
  da entrada. Ficam como **Proposta no SDD 18**, até o merge.

## Decisões

Cada uma é caso do `AGENTS.md` (2, 3 ou 6). **Aprovadas pelo dono em 2026-11-24, todas com a
recomendação** (o merge do #46); as opções descartadas estão no corpo do #46.

| # | Decisão | Caso | Recomendação |
| --- | --- | --- | --- |
| 1 | Roteiro: M2 logo depois da fase 4; aarch64 com o timedemo no M2 | 3 (roteiro da escada) e 6 | M2 após a 4; aarch64 fecha só o timedemo |
| 2 | doomgeneric: baixar commit fixo com checksum, ou vendorizar | 2 | (a) baixar |
| 3 | clang como ferramenta de build; sem clang o M2 é opcional | 2 | Opcional até o M2 fechar |
| 4 | WAD: Freedoom no CI e no timedemo, `doom1.wad` como opcional local | 2 | (c) Freedoom mais `doom1.wad` local |
| 5 | Framebuffer como VMO direto, `kind` 5; posse pelo mapeamento ou pelo handle | 6 | (a) VMO direto; posse pelo mapeamento |
| 6 | Entrada como canal, `kind` 6; código HID; sem release no aarch64 | 6 | HID (`hal::KeyCode`); aarch64 sem entrada |
| 7 | Libc como `user/libc`, com exceção na regra 11 | 3 | (a) `user/libc` com a exceção |

## Como verifiquei

- **Interno (lido no repositório, commit `7188c25` da base):** `AGENTS.md`; `docs/sdd/08_roadmap.md`
  (linhas da fase 5, escada e decisões futuras); `docs/sdd/17_processos_isolamento_e_syscalls.md`
  (ABI, mensagem de boot, `ProcessEvent`, decisões do dono); `docs/sdd/11_teclado.md`;
  `docs/sdd/16_terminal.md`; `docs/guidelines/03_camadas_e_dependencias.md` (regras 11 e 12);
  `docs/guidelines/04_unsafe_e_asm.md`; `docs/tech-debt/` (numeração e os itens 0027–0032).
- **Terceiros, com link:**
  - [`ozkl/doomgeneric`](https://github.com/ozkl/doomgeneric): licença (metadado da API, commit do clone
    `e257a095`), `doomgeneric/doomgeneric.h`, `doomgeneric.c`, `g_game.c`, `d_main.c`, `i_system.c`,
    `w_file_stdc.c`, `doomfeatures.h`, `Makefile`; README.
  - [`freedoom/freedoom`](https://github.com/freedoom/freedoom) (commit `e4ec24d`): `COPYING.adoc`,
    `lumps/README.DEMO`, `buildcfg.txt`; release `v0.13.0` (nomes dos assets).
  - [`llvm/llvm-project`](https://github.com/llvm/llvm-project), `clang/include/clang/Options/Options.td`
    (`master`, lido pela API): `ffreestanding`, `nostdlib`, `nostdinc`, `target`.
  - Clang local (Apple clang 21): `clang --help` (secundária).
- **NÃO VERIFICADO (marcado no texto):** o texto do `LICENSE` do doomgeneric; o WAD construído do
  Freedoom e o lump `DEMO1` dentro dele; as licenças das músicas do Freedoom; o efeito exato de
  `-nostdlib` e `-nostdinc` e o triple do clang para os dois alvos; a presença do `rust-lld`; o
  `fbdev` do guest Linux de medida; a viabilidade de Windows na QEMU; o comportamento de
  `D_DoomLoop` no timedemo; a leitura jurídica da GPL-2.0 para o `doom`.
- **Não rodei nada:** nenhum build, nenhum download de WAD ou de tarball, nenhum teste na QEMU.

## Dívidas

Nova na fatia 1a: [DEBT-0159](../tech-debt/0159-framebuffer-com-outra-ordem-de-canais-sem-vmo.md) (framebuffer com
outra ordem de canais fica sem `kind` 5). As que o desenho toca já existem: [DEBT-0057](../tech-debt/0057-tsc-entre-nucleos-sem-verificacao.md)
(TSC entre núcleos, que pesa na medida de tempo), [DEBT-0051](../tech-debt/0051-redesenho-da-tela-com-irq-mascarada.md)
(o redesenho com IRQ mascarada, que a posse da tela usa), e as de teclado já listadas no SDD 11
(sem teclado no aarch64, [DEBT-0050](../tech-debt/0050-historico-sem-rolagem-no-aarch64.md)).

---

**Ao terminar:** acrescente a linha no índice do [`README.md`](README.md) da pasta, e lembre que este
capítulo muda **no mesmo PR** que tornar seu conteúdo passado.
