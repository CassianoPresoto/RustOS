# SDD 12 — Memória física e heap

> **O que este capítulo responde:** como o `core/mm` entrega frames e bytes (buddy,
> cache por CPU, heap), por que cada escolha, e o que o `kernel/` precisa ligar.

Estado: o núcleo puro está em `core/mm`, testado no host, e o kernel o liga: o buddy
entrega os frames e o heap é o `GlobalAlloc` (fase 2 do [roadmap](08_roadmap.md)). O
cache por CPU está ligado desde o SMP (fase 3). Os números medidos estão no
[SDD 07](07_desempenho_e_recursos.md).

## As camadas

| Tipo | Arquivo | Papel | Metadado |
| --- | --- | --- | --- |
| `BuddyAllocator` | `buddy.rs` | Blocos de 2^k frames alinhados a 2^k, fusão com o irmão, corrida contígua | 2,03 bits/frame |
| `FrameCache<CAP>`, `FrameCaches<CPUS, CAP>` | `cache.rs` | Pilha de frames de ordem 0 por CPU, em lote sobre o buddy | `CAP` × 8 bytes por CPU |
| `KernelHeap<M>` | `heap.rs` | Slabs de uma página por classe de tamanho; o resto em páginas do buddy | 64 bytes por slab, dentro da página |
| `BitTree` | `bittree.rs` | Bitmap hierárquico (interno) | — |

O `BitmapFrameAllocator` saiu do `core/mm` quando o kernel passou ao buddy:
dois alocadores de frames no mesmo crate eram a dívida que isso paga, e o
bitmap perdia onde o kernel precisa (bloco contíguo por varredura O(frames), 1 240 ns
para 2 MiB contra 33 ns). Ficou uma cópia só de teste, `core/mm/src/bench_bitmap.rs`,
como base de comparação do `bench.rs`. O preço é o dobro de metadado: 8 144 para
16 704 bytes em 254 MiB.

Duas portas, as duas definidas em `core/mm`, no padrão do `TableMemory` do `hal`:

- **`FrameMemory`** (`memory.rs`): `read_u64`, `write_u64` e `zero` sobre endereço
  físico. O teste a implementa com um `Vec`; o kernel, com o mapa direto
  (`físico + direct_map_offset`). Só o heap a usa.
- **`FrameAllocator`** (`buddy.rs`): `allocate_contiguous(count)` e
  `deallocate_contiguous(first, count)`. O buddy a implementa; `Cached` (o cache
  sobre o buddy) também. O heap pede frames por ela e não sabe qual dos dois é.

## O buddy

- **Ordens 0 a 18** (`MAX_ORDER`): o maior bloco é 1 GiB, a página grande do x86_64
  e do aarch64; 2 MiB é a ordem 9. Ordem a mais custa quase nada: o bitmap de cada
  ordem tem metade do anterior. O preço é a fusão em cascata de até 18 passos no
  pior caso (medido no SDD 07).
- **Alinhamento absoluto.** O irmão de um bloco é `frame ^ (1 << ordem)` em número de
  frame físico, e o bitmap de cada ordem começa em `base >> ordem`. Região que não
  começa alinhada vira blocos alinhados nas pontas, sem gastar bitmap abaixo de `base`.
- **Metadado:** um bitmap por ordem, onde bit ligado é "bloco livre e não fundido",
  mais um resumo hierárquico (um bit por palavra não vazia, até 5 níveis). O menor
  bloco livre de uma ordem sai em um passo por nível, e uma máscara `nonempty` diz
  quais ordens têm bloco livre. **A memória dos frames livres nunca é tocada.**
- **Capacidade fixa:** a fatia de `u64` vem de fora; `words_needed(frames)` é um
  limite superior para qualquer `base`. Acima de 2^30 frames (4 TiB) `new` recusa.
- **Menor endereço primeiro**, na menor ordem que serve: a memória livre se concentra
  embaixo.
- **Dupla liberação detectada** (`FrameError::DoubleFree`): o bloco, um pai livre ou
  qualquer filho livre. Bloco desalinhado à ordem, fora da região ou de ordem acima
  de 18 também erra. Liberar com a ordem errada para menos vaza; para mais é pego se
  tocar frame livre.
- **`allocate_contiguous(count)`** serve qualquer `count`: pega o bloco da ordem que
  o contém e devolve a sobra. `deallocate_contiguous` aceita qualquer corrida de
  frames ocupados, não só as que saíram dele.
- **`release_range` e `reserve_range`** mantêm a semântica do bitmap: liberar só os
  frames inteiros, repetir é inócuo; reservar todo frame tocado, partindo os blocos.
  `reserve_range` custa O(frames × ordens): é de boot.

### Por que bitmaps e não listas nos frames livres

Medi as duas formas no host (SDD 07): a forma (b), listas duplamente encadeadas dentro
dos blocos livres por `FrameMemory`, mais um bit por bloco para achar o irmão.

| | (a) bitmaps | (b) listas intrusivas |
| --- | --- | --- |
| Metadado | 2,03 bits/frame | o mesmo bitmap (precisa dele para o irmão e a dupla liberação) |
| Toca a memória livre | nunca | a cada operação, 2 palavras do bloco e dos vizinhos |
| Em lote | 9,8 ns/op | 13,6 ns/op |
| Um frame, anel de 4096 vivos | 21 ns | 14 ns |
| Bloco de 2 MiB | 33 ns | 37 ns |

A (b) é mais rápida só onde o caminho de uma operação é curto, e o cache por CPU já
cobre esse caso. A (a) não exige que a memória livre esteja mapeada nem escrevível,
não deixa ponteiro em página livre (a página livre fica zerada se foi zerada) e uma
escrita depois da liberação não corrompe o alocador. Por isso a (a) está no
kernel; a (b) existe só em `core/mm/src/bench.rs`, como referência de medição.

## O cache por CPU

`FrameCache<CAP>` é uma pilha de até `CAP` frames de ordem 0. Vazia, `allocate` pede
`CAP / 2` frames ao buddy; cheia, `deallocate` devolve `CAP / 2`. O caminho quente
não toca o buddy. `FrameCaches<CPUS, CAP>` guarda um cache por CPU, com `CPUS` fixo em
compilação; `as_mut_slice` entrega cada um à sua CPU.

O núcleo não trava nada: **quem chama garante que só uma CPU usa cada cache**, e que o
buddy, que é compartilhado, está sob lock enquanto o cache fala com ele. `Cached`
(`cache.through(&mut buddy)`) faz o cache servir `FrameAllocator` e `FrameSource`:
um frame passa pelo cache; uma corrida de mais de um frame vai direto ao buddy.

Limite: dupla liberação de um frame que está no cache só aparece quando o frame sai
dele para o buddy (DEBT-0017).

## O heap

`KernelHeap<M: FrameMemory>` é a base do `GlobalAlloc`:

```rust
pub fn allocate<F: FrameAllocator>(&mut self, layout: Layout, frames: &mut F) -> Option<PhysAddr>;
pub fn deallocate<F: FrameAllocator>(&mut self, ptr: PhysAddr, layout: Layout, frames: &mut F)
    -> Result<(), HeapError>;
```

O endereço é físico: o kernel soma o deslocamento do mapa direto para entregar o
ponteiro, e o subtrai em `dealloc`. `deallocate` recebe o mesmo `Layout` de
`allocate`: dele saem a classe e o número de páginas, sem metadado por objeto.

- **Classes: 16, 32, 64, 128, 256, 512 e 1024 bytes** (`MAX_CLASS`). O menor é 16
  porque o mapa de livres do cabeçalho tem 256 bits, um por objeto. O objeto
  fica em offset múltiplo do próprio tamanho, então o alinhamento do `Layout` até a
  classe é de graça; `max(size, align)` escolhe a classe.
- **Por que não há classe de 2048:** o cabeçalho de 64 bytes no início da página
  tira um objeto de 2 KiB, e sobra um por página, 50% de desperdício, o mesmo de pedir
  uma página inteira ao buddy. Classe de 2048 não ganharia nada.
- **Slab de uma página.** Cabeçalho de 8 palavras: `next` e `prev` (slabs com vaga da
  classe), `free` (lista de objetos devolvidos), `state` (usados e `bump`) e o mapa de
  livres, 4 palavras com um bit por objeto. Objeto
  nunca entregue vem do `bump`, sem pré-encadear a página. Slab cheio sai da lista;
  slab vazio volta ao buddy, menos o último da classe (histerese contra ir e voltar).
- **Pedido acima de 1024 bytes** (ou alinhamento acima dele) vai em
  `ceil(max(size, align) / 4096)` frames contíguos do buddy, alinhados à potência de
  dois que os contém.
- **Dupla liberação no slab** é pega pelo **bit de livre do objeto**, no mapa do
  cabeçalho (posição `deslocamento / tamanho`): ligado ao devolver, desligado ao
  entregar. A marca não vem de dado do usuário: um objeto vivo pode guardar qualquer
  valor em qualquer palavra, inclusive o que antes servia de marca (a revisão do PR 5
  achou que um objeto vivo com esse valor no deslocamento 8 dava pânico no `dealloc`).
  `allocate` zera a palavra do encadeamento (não vaza ponteiro). Ponteiro que não
  cai em objeto entregue (cabeçalho, meio de objeto, objeto nunca entregue) dá
  `BadPointer`.
- **Zerar ao liberar:** `KernelHeap::new(memory, zero_on_free)` e `set_zero_on_free`.
  Com a política ligada, o objeto é zerado antes do encadeamento e a página,
  inteira, **antes** de voltar para o buddy: depois dele outra CPU já poderia tê-la
  pegado. Antes de zerar, `FrameAllocator::verify_used` confere que o bloco está
  ocupado no buddy, para a dupla liberação de bloco livre não gravar zero em lugar
  nenhum (se outro dono já pegou o bloco, o buddy não tem como saber). A política é do heap; frame que sai do buddy para
  processo é zerado por quem o usa (`FrameMemory::zero`). No kernel, `DirectMap::zero` manda
  faixas de 512 bytes ou mais, múltiplas de 64, ao `arch/` (`DC ZVA` no aarch64; no x86_64
  ainda o `memset`), e o resto ao `write_bytes`. Custo no [SDD 07](07_desempenho_e_recursos.md).

| Classe | Objetos por slab | Aproveitamento da página |
| --- | --- | --- |
| 16 | 252 | 98,4% |
| 32 | 126 | 98,4% |
| 64 | 63 | 98,4% |
| 128 | 31 | 96,9% |
| 256 | 15 | 93,8% |
| 512 | 7 | 87,5% |
| 1024 | 3 | 75,0% |

## Como o kernel liga

| Peça | Arquivo | O que é |
| --- | --- | --- |
| Frames | `kernel/src/frames.rs` | `static FRAMES: IrqLock<Option<BuddyAllocator<'static>>>` e o `Frames`, um tipo sem dados e `Copy`. Só `install` o cria, então ele prova que o alocador existe. Cada método trava `FRAMES` só pela duração da chamada. Implementa `FrameSource` e `FrameAllocator`, e é o `Memory::frames` |
| `FrameMemory` | `kernel/src/heap.rs` | `DirectMap`: `físico + deslocamento do mapa direto`. O `unsafe` é o `DirectMap::new` (todo endereço é de frame do kernel, alinhado e visível no mapa direto); os métodos da porta são seguros |
| Heap | `kernel/src/heap.rs` | `static HEAP: IrqLock<Option<State>>` com `KernelHeap<DirectMap>`, o `Frames` e a conta de bytes; `#[global_allocator]` |

`Memory::init` faz, na ordem de hoje: `BuddyAllocator::new` sobre o bloco que
`find_free_block` achou (o intervalo coberto é o `managed_span` de `summarize`, que inclui
as regiões `BootloaderReclaimable`), `release_range` das regiões utilizáveis,
`reserve_range` do próprio metadado, `frames::install` e `heap::install`. As regiões
recuperáveis entram depois, em `Memory::reclaim_bootloader` ([SDD 13](13_memoria_virtual.md)).

**Ordem de lock: `HEAP`, depois `FRAMES`.** Só o heap trava os dois, nessa ordem,
e quem segura `FRAMES` (só dentro de um método de `Frames`) nunca chama o heap. Os
dois são `IrqLock`, spinlocks de verdade (a máscara de interrupção só protege contra o handler da mesma
CPU; contra outra CPU vale o `compare_exchange` do `SpinLock`), e a disputa entre CPUs é medida
([SDD 07](07_desempenho_e_recursos.md)). A ordem entre os locks do kernel todo está no [SDD 15](15_tarefas_e_preempcao.md).

- **`alloc`** chama `KernelHeap::allocate` e soma o deslocamento do mapa direto ao
  `PhysAddr`; devolve nulo se o heap não foi instalado ou não há frames.
- **`dealloc`** subtrai o deslocamento e chama `deallocate` com o mesmo `Layout`. `Err`
  (ponteiro que não é do heap, dupla liberação) é defeito do kernel: pânico com o
  ponteiro, o `Layout` e o erro, depois de soltar o lock.
- **`heap::stats()`** dá `pages` (`KernelHeap::pages_held`: slabs mais blocos de
  páginas) e `used_bytes`: soma dos `Layout::size` pedidos e ainda não devolvidos, sem o
  arredondamento da classe. É o que `mem` mostra na linha `heap`.
- **`heap::self_test()`** roda no boot e imprime `[rustos] heap self-test: ok`: `Vec<u8>`
  de 15 tamanhos (1 byte a 70 000, com as bordas 1024/1025 e 4096/4097), `Box` de
  `u64`, `[u32; 300]` e dois tipos alinhados a 64 e a 4096, um `Vec<u32>` que cresce até
  10 000, e a contabilidade de volta ao ponto de partida. Depois aloca um bloco de duas
  páginas, o solta, lê as palavras dele pelo mapa direto (voltam zeradas) e confere
  que os frames voltaram ao buddy. Depois zera faixas de começo e tamanho variados (512 B até
  3 páginas, sem alinhar ao bloco do `DC ZVA`) numa área de 3 páginas cheia de `0xAA` e
  confere que os vizinhos de cada faixa seguem intactos.
- **Zerar ao liberar está ligado** (`ZERO_ON_FREE`). O custo no alvo está no
  [SDD 07](07_desempenho_e_recursos.md).
- **Só o kernel usa `alloc`**: `extern crate alloc` está em `kernel/src/main.rs`, e
  `core/` continua sem chamar o alocador global (guidelines 05).

### O cache por CPU está ligado

Decisão da segunda onda da fase 3, com número ([SDD 07](07_desempenho_e_recursos.md)): com todas as CPUs alocando e devolvendo frames, o lock do buddy é disputado e o cache por CPU
tira a disputa do caminho. `Frames::allocate`, `deallocate` e as corridas de **um** frame (`allocate_contiguous(1)`, o que o heap pede para o slab)
passam pelo `FrameCache<32>` do `PerCpu` da CPU que roda, atrás do `IrqLock` próprio dele (sem disputa: só a CPU dona o pega). Corrida de mais de um frame vai direto ao buddy.

- **Ordem de lock:** heap, cache da CPU, frames. O buddy por trás do cache (`Backing`) trava `FRAMES` por chamada; quem segura `FRAMES` não chama o heap nem o cache.
- **`free_frames`** soma o buddy e os caches de todas as CPUs: `mem` e `frames:` do boot seguem verdadeiros. Os frames de um cache não estão no buddy: de lá só voltam quando o cache enche (metade vai) ou esvazia (metade vem).
- **O que se perde:** `verify_used` de um frame só devolve `Ok` sem conferir com o cache ligado ([DEBT-0017](../tech-debt/0017-dupla-liberacao-escondida-no-cache-de-frames.md)).
- **Custo:** 256 bytes por CPU (`CACHE_CAP` = 32 × 8). Memória parada em cache: até 31 frames (124 KiB) por CPU.

O heap segue com um lock global: com 4 CPUs alocando sem parar ele disputa ([DEBT-0054](../tech-debt/0054-heap-com-lock-global.md)).
