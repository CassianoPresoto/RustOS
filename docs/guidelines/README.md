# Guidelines — como se escreve aqui

**Prescritivo**: estilo, nomenclatura, arquitetura. O descritivo — o que o
projeto é e como se trabalha nele — mora em [`../sdd/`](../sdd/README.md).

## Como os arquivos funcionam

Um tema por arquivo, nomeado `NN_tema.md`. O número é **ordem de leitura**, não
identidade. Sem frontmatter. Tema novo nasce de [`_template.md`](_template.md).

Regra aqui é curta, no imperativo e **verificável**: se ninguém consegue apontar
uma violação em review, não é regra — é preferência, e preferência não entra. O
teste prático: dá para checar com `grep` ou com um `cargo xtask check-*`?

Regra nasce aqui no PR que a decide, listada na seção *Decisões* dele; mudar ou
tirar regra é um dos casos em que o agente para ([`AGENTS.md`](../../AGENTS.md#decisões)).

## Índice

| Tema | Cobre |
| --- | --- |
| [`01_estilo.md`](01_estilo.md) | `rustfmt` e `clippy` pedantic como fonte da verdade, aviso vira erro, e como abrir exceção |
| [`02_nomenclatura.md`](02_nomenclatura.md) | Nome de crate, idioma do código e da prosa, nomes de teste, mensagens do kernel |
| [`03_camadas_e_dependencias.md`](03_camadas_e_dependencias.md) | Quem pode depender de quem; nenhum crate externo; o que o `check-layers` confere |
| [`04_unsafe_e_asm.md`](04_unsafe_e_asm.md) | Onde `unsafe` e `asm!` podem morar, `// SAFETY:`, invariante no construtor |
| [`05_recursos_e_desempenho.md`](05_recursos_e_desempenho.md) | Memória e CPU: tamanho fixo no núcleo, nada de alocação no caminho quente, otimização com número |
| [`06_comentarios.md`](06_comentarios.md) | O que um comentário pode dizer, 4 linhas por bloco, `DEBT-NNNN` |
| [`07_testes.md`](07_testes.md) | O que precisa de teste e em que andar; o smoke das duas ISAs |
| [`08_commits_branches_e_pr.md`](08_commits_branches_e_pr.md) | Formato de commit, branch e PR; o que o PR precisa dizer |
