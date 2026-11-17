# SDD — o projeto e como se desenvolve nele

**Descritivo**: como o sistema é hoje e como se trabalha nele. O prescritivo —
como escrever, nomear, estruturar — mora em [`../guidelines/`](../guidelines/README.md).

A regra curta que separa os dois: **se descreve como é, é SDD; se manda fazer,
é guidelines.**

## Como os arquivos funcionam

Um capítulo por arquivo, nomeado `NN_topico.md`. O número é **ordem de leitura**,
não identidade. Sem frontmatter. Capítulo novo nasce de [`_template.md`](_template.md).

**O SDD é a fonte da verdade sobre o estado do projeto.** Decisão entra aqui no PR
que a toma; se o código e o SDD divergirem, um dos dois está errado — corrija no
mesmo PR.

## Índice

| Capítulo | O que responde |
| --- | --- |
| [`01_visao_e_objetivos.md`](01_visao_e_objetivos.md) | O que o RustOS quer ser, as metas que se medem e o que fica de fora |
| [`02_arquitetura.md`](02_arquitetura.md) | O hexágono: centro, portas, adaptadores, raiz de composição e o caminho de uma chamada |
| [`03_mapa_de_codigo.md`](03_mapa_de_codigo.md) | Onde ponho isto? Árvore comentada e quem depende de quem |
| [`04_boot.md`](04_boot.md) | Do firmware ao `boot ok`: Limine, estado da máquina, sequência do `kmain` e layout de memória |
| [`05_build_e_execucao.md`](05_build_e_execucao.md) | Toolchain, alvos, perfis, `make`/`xtask`, QEMU e firmware |
| [`06_testes.md`](06_testes.md) | Os andares de teste, onde cada um roda e o que ainda não existe |
| [`07_desempenho_e_recursos.md`](07_desempenho_e_recursos.md) | O que já se mede, como comparar com Linux e Windows, e as técnicas que a pesquisa aponta |
| [`08_roadmap.md`](08_roadmap.md) | A fila de implementação: as fases, o que cada uma entrega e quando termina |
| [`09_seguranca.md`](09_seguranca.md) | Modelo de ameaça, mecanismos por fase com custo e fonte, e as decisões de segurança abertas |
| [`10_interrupcoes.md`](10_interrupcoes.md) | Exceções, IRQ e tempo nas duas ISAs: sequência de init, vetores, controladores, timer, relógio e fontes |
| [`11_teclado.md`](11_teclado.md) | Da tecla do PC à `Key` do terminal: i8042, decodificador, layouts US e ABNT2, teclas mortas |
| [`12_memoria_fisica_e_heap.md`](12_memoria_fisica_e_heap.md) | Buddy, cache de frames por CPU e heap de slabs: o desenho, as escolhas e o que o kernel liga |
| [`13_memoria_virtual.md`](13_memoria_virtual.md) | Tabelas próprias, W^X, pilha própria, memória do bootloader devolvida e KASLR: o desenho, as medidas e as escolhas |
| [`14_escalonador.md`](14_escalonador.md) | A porta `Scheduler` e as políticas `RoundRobin`, `Eevdf`, `Fifo` e `Composite` (o kernel usa o `Composite`): fórmulas, fontes, medidas e o que o kernel precisa saber |
| [`15_tarefas_e_preempcao.md`](15_tarefas_e_preempcao.md) | Tarefas, troca de contexto e preempção: o escalonador, o desenho por ISA, a regra de lock, o estado de FPU/SIMD e as medidas |
