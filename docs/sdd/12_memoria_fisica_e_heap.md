# SDD 12 — Memória física e heap

> **O que este capítulo responde:** como o `core/mm` entrega frames e bytes (buddy,
> cache por CPU, heap), por que cada escolha, e o que o `kernel/` precisa ligar.

Estado: o núcleo puro está em `core/mm` e testado no host. O kernel ainda usa o
`BitmapFrameAllocator` e não tem `GlobalAlloc` (DEBT-0016); a ligação é a fase 2 do
[roadmap](08_roadmap.md). Os números medidos estão no [SDD 07](07_desempenho_e_recursos.md).

## As camadas

| Tipo | Arquivo | Papel | Metadado |
| --- | --- | --- | --- |
| `BitmapFrameAllocator` | `frame.rs` | Alocador de boot: 1 bit por frame, varredura | 1 bit/frame |
| `BuddyAllocator` | `buddy.rs` | Blocos de 2^k frames alinhados a 2^k, fusão com o irmão, corrida contígua | 2,03 bits/frame |
| `FrameCache<CAP>`, `FrameCaches<CPUS, CAP>` | `cache.rs` | Pilha de frames de ordem 0 por CPU, em lote sobre o buddy | `CAP` × 8 bytes por CPU |
| `KernelHeap<M>` | `heap.rs` | Slabs de uma página por classe de tamanho; o resto em páginas do buddy | 32 bytes por slab, dentro da página |
| `BitTree` | `bittree.rs` | Bitmap hierárquico (interno) | — |

Duas portas, as duas definidas em `core/mm`, no padrão do `TableMemory` do
`arch/aarch64/src/walk.rs`:

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
  porque o objeto livre guarda o encadeamento e a marca de livre (2 palavras). O objeto
  fica em offset múltiplo do próprio tamanho, então o alinhamento do `Layout` até a
  classe é de graça; `max(size, align)` escolhe a classe.
- **Por que não há classe de 2048:** o cabeçalho de 32 bytes no início da página
  tira um objeto de 2 KiB, e sobra um por página, 50% de desperdício, o mesmo de pedir
  uma página inteira ao buddy. Classe de 2048 não ganharia nada.
- **Slab de uma página.** Cabeçalho de 4 palavras: `next` e `prev` (slabs com vaga da
  classe), `free` (lista de objetos devolvidos) e `state` (usados e `bump`). Objeto
  nunca entregue vem do `bump`, sem pré-encadear a página. Slab cheio sai da lista;
  slab vazio volta ao buddy, menos o último da classe (histerese contra ir e voltar).
- **Pedido acima de 1024 bytes** (ou alinhamento acima dele) vai em
  `ceil(max(size, align) / 4096)` frames contíguos do buddy, alinhados à potência de
  dois que os contém.
- **Dupla liberação no slab** é pega pela marca de livre na segunda palavra do objeto,
  que `allocate` zera (as duas palavras: também não vaza ponteiro). Ponteiro que não
  cai em objeto entregue (cabeçalho, meio de objeto, objeto nunca entregue) dá
  `BadPointer`.
- **Zerar ao liberar:** `KernelHeap::new(memory, zero_on_free)` e `set_zero_on_free`.
  Com a política ligada, o objeto é zerado antes do encadeamento e a página, ao
  voltar para o buddy, inteira. A política é do heap; frame que sai do buddy para
  processo é zerado por quem o usa (`FrameMemory::zero`). Custo no
  [SDD 07](07_desempenho_e_recursos.md).

| Classe | Objetos por slab | Aproveitamento da página |
| --- | --- | --- |
| 16 | 254 | 99,2% |
| 32 | 127 | 99,2% |
| 64 | 63 | 98,4% |
| 128 | 31 | 96,9% |
| 256 | 15 | 93,8% |
| 512 | 7 | 87,5% |
| 1024 | 3 | 75,0% |

## O que o kernel precisa ligar

1. **`FrameMemory` pelo mapa direto:** um adaptador em `kernel/src/memory.rs` com
   `direct_map_offset`; `read_u64`/`write_u64` com `read_volatile`/`write_volatile` ou
   ponteiro comum (a memória é do kernel), `zero` com `write_bytes`. É o `unsafe`
   desta etapa, com o invariante no construtor.
2. **`BuddyAllocator` no lugar do bitmap:** `words_needed`, `new`, `release_range`
   das regiões utilizáveis e `reserve_range` do bloco do próprio metadado, como hoje.
   `allocate`, `deallocate`, `allocate_contiguous`, `free_frames` e `total_frames` têm
   as mesmas assinaturas; `FrameSource` também.
3. **Locks:** um lock para o buddy; um cache por CPU, sem lock, acessado só com as
   interrupções mascaradas na CPU dona; um lock para o `KernelHeap` (ou um heap por
   CPU, depois de medir). A ordem é sempre heap, depois buddy.
4. **`GlobalAlloc`:** `alloc` chama `KernelHeap::allocate` e soma o deslocamento;
   `dealloc` subtrai e chama `deallocate`, e entra em pânico no `Err`.
