---
id: DEBT-0142
title: O objeto do kernel e a lista de trabalho ainda alocam sem `try_*` sob o teto em bytes
status: Open
date: 2026-11-19
trigger: O primeiro programa de usuário que não seja nosso, ou um teste com o heap cheio que derrube o kernel
paid_date:
---

# DEBT 0142 — O objeto do kernel e a lista de trabalho ainda alocam sem `try_*` sob o teto em bytes

## O que falta

O teto de `PROCESS_BYTES_MAX` por processo (cobrado ao criador) limita o que um processo pode ter no heap, e as cobranças de ponta, mensagem, VMO, processo e thread dão `NoMemory` antes de alocar. Mas o `Arc::new` do objeto (`ChannelEnd`, `ThreadObj`, `Process`, `VmarObj`), o `Arc` do cabeçalho da mensagem, a lista de trabalho do fechamento (`Work`) e o `Box<Start>` da thread seguem pela alocação que aborta. Com o heap cheio por outra causa (promessas de frame vencidas, falta de RAM) a criação derruba o kernel em vez de dar `NoMemory`. O payload da mensagem e os slots da tabela e do VMO já usam `try_reserve`.

## Por que foi aceito

A toolchain estável não tem `Arc::try_new`, e o teto por processo, a soma dos processos vivos (no máximo 24 tarefas) e a promessa de frames deixam o aborto longe do uso normal; nenhum teste enche o heap de propósito.

## Gatilho

O primeiro programa de usuário que não seja nosso, ou um teste com o heap cheio que espere `NoMemory` em cada chamada.

## Como pagar

Uma caixa com contagem própria e `alloc` fracassável para os objetos (`Arc` próprio com `try_new`), `Work` com capacidade reservada antes de pegar o lock, e um teste com o heap cheio que espera `NoMemory` em cada chamada.
