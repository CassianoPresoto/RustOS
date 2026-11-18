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
- **Nada de alocação, `format!` ou laço sem limite no caminho quente** (interrupção,
  escalonador, syscall). Formatação é só para o console.
- **Nada de ponto flutuante no kernel.** Os alvos são soft-float de propósito: só a
  tarefa criada com `simd: true` salva e restaura FPU/SIMD na troca de contexto.
- **Metadado por recurso tem custo escrito:** quem cria estrutura por frame, por
  tarefa ou por página diz no SDD 07 quantos bytes ela custa por unidade.
- **Otimização entra com número:** o PR traz o comando, o antes e o depois
  (`make size`, linha do boot ou benchmark), na mesma máquina e perfil.
- **Benchmark de latência só com aceleração de hardware (KVM/HVF) ou em metal.**
  Número de TCG serve para comparar o RustOS consigo mesmo, nunca com outro sistema.
- **Estado por CPU antes de lock global**, quando houver SMP.

## Como se verifica

```sh
grep -rn 'f32\|f64' --include=*.rs core lib arch boot drivers kernel
grep -rn 'extern crate alloc\|alloc::' --include=*.rs core
```

`make size` no PR que mexe em `kernel/`, `arch/`, `boot/` ou `drivers/`.
