# Guidelines 09 — Segurança

> **O que este tema cobre:** as regras de segurança que valem em todo PR. O modelo
> de ameaça e os mecanismos por fase estão no [SDD 09](../sdd/09_seguranca.md).

## Regras

- **Dado de fora do kernel vira tipo do núcleo só por função de tradução com
  teste do caso inválido.** Vale para resposta do bootloader, registrador de
  dispositivo, pacote, bloco de disco e argumento de syscall. Exemplo:
  `framebuffer_info` em `boot/limine` e os testes que recusam formato.
- **Segredo não vai para `kprintln!`, log, serial nem `Debug`.** Isso cobre senha,
  chave e semente do aleatório. Tipo que guarda segredo implementa `Debug` à mão
  e escreve `<redacted>`.
- **Proteção de hardware que a CPU tem e o kernel não liga vira dívida** com
  gatilho. O relatório de boot diz o que ficou ligado.
- **Mecanismo de segurança desligado por desempenho exige o número** no SDD 07,
  do mesmo jeito que otimização (guidelines 05).
- **Nenhuma página do kernel é gravável e executável ao mesmo tempo**, a partir da
  fase 2. O teste dentro do kernel confere.

## Como se verifica

- **Tradução de dado externo:** em review, sem comando automático. Toda função que
  recebe ponteiro, registrador ou bytes de fora tem teste com entrada inválida no
  mesmo PR.
- **Segredo:** heurística, que acha `derive(Debug)` em tipo com nome de segredo
  (até 4 linhas entre o `derive` e o tipo). O review confere o resto, como
  `impl Debug` à mão que não escreva `<redacted>`.

  ```sh
  grep -rn -A4 'derive(.*Debug' --include='*.rs' core lib arch boot drivers kernel \
    | grep -iE '(struct|enum) \w*(secret|password|seed|token)'
  ```

- **Proteção ligada:** a partir da fase 1, o `make smoke` passa a exigir a linha
  `[rustos] protections:` (SDD 08, fase 1).
- **Segurança desligada por desempenho:** em review. O PR que desliga um mecanismo
  traz o número de antes e depois, como no guidelines 05.
- **W^X:** o autoteste do boot percorre as tabelas do kernel e o `make smoke` exige a
  linha `[rustos] w^x: ok`; `fault text` no shell confere que a CPU recusa escrever
  em `.text` ([SDD 13](../sdd/13_memoria_virtual.md)).
