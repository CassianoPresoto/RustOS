# Guidelines 05 — Recursos e desempenho

> **O que este tema cobre:** as regras que mantêm o kernel pequeno e rápido, e como
> uma otimização prova que vale. O método e as fontes estão no
> [SDD 07](../sdd/07_desempenho_e_recursos.md).

## Regras

- **Nenhum crate externo** (guidelines 03, regra 7). Cada linha que roda no kernel
  está neste repositório e cabe no `make size`.
- **Estrutura do núcleo tem capacidade fixa ou recebe a memória de fora.** `RunQueue<N>`
  usa `const N`; `BuddyAllocator` recebe a fatia. Nada em `core/` chama o
  alocador global.
- **Nada de alocação, `format!` ou laço sem limite no caminho quente**: interrupção,
  escalonador e a syscall que não cria nada (a nula, a de espera, o fechamento de handle).
  Formatação é só para o console.
- **A syscall que cria objeto ou handle, ou que carrega mensagem de IPC, pode alocar do heap, e só
  com `try_*`:** a falta de memória vira `NoMemory` e nunca derruba o kernel. Vale para
  `channel_create`, `channel_write`, `handle_duplicate` e as de criação que vierem. O que ainda
  usa alocação que aborta é dívida registrada ([DEBT-0142](../tech-debt/0142-objeto-do-kernel-sem-try-new.md)).
- **Nada de ponto flutuante no kernel.** Os alvos são soft-float de propósito: só a
  tarefa criada com `simd: true` salva e restaura FPU/SIMD na troca de contexto.
- **Metadado por recurso tem custo escrito:** quem cria estrutura por frame, por
  tarefa ou por página diz no SDD 07 quantos bytes ela custa por unidade.
- **Otimização entra com número:** o PR traz o comando, o antes e o depois
  (`make size`, linha do boot ou benchmark), na mesma máquina e perfil.
- **Benchmark de latência só com aceleração de hardware (KVM/HVF) ou em metal.**
  Número de TCG serve para comparar o RustOS consigo mesmo, nunca com outro sistema.
- **Estado por CPU antes de lock global**, quando houver SMP.
- **Boot normal e boot de teste.** Autoteste ou medida que espera relógio (janela de
  tempo), roda tarefas ou mede **só roda com `rustos.selftest=1`**. Entra no boot
  normal o que prova algo que nada mais exercita e custa menos de ~2 ms em TCG. O PR
  que acrescenta autoteste ao boot diz o custo nos dois modos e em qual ele roda.
- **Cada linha do boot custa tempo na serial.** O modo normal imprime só o que um gate
  confere ou o que avisa; o detalhe fica no modo de teste e nos comandos do shell.

## Como se verifica

`make smoke-fast ARCH=all PROFILE=release` imprime o tempo do kernel até o prompt; PR que mexe
no boot traz o antes e o depois dele (a linha `boot time:`).

```sh
grep -rn 'f32\|f64' --include=*.rs core lib arch boot drivers kernel
grep -rn 'extern crate alloc\|alloc::' --include=*.rs core
```

`make size` no PR que mexe em `kernel/`, `arch/`, `boot/` ou `drivers/`.
