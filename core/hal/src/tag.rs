//! Etiqueta de TLB de um espaço de usuário: PCID no x86_64, ASID no aarch64 (SDD 17, "CR3 e TTBR0
//! por processo, com PCID e ASID"). Quem decide a etiqueta e o esvaziamento é o kernel; quem grava
//! o `CR3` e o `TTBR0` é o `arch/`.

/// O que a troca esvazia nesta CPU antes de carregar a etiqueta.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagFlush {
    /// Nada: as entradas que a CPU tem continuam valendo.
    Keep,
    /// Só as entradas desta etiqueta: o espaço mudou desde que esta CPU o viu (migração).
    Space,
    /// Tudo, inclusive o que não é global: a etiqueta é de outra geração (regra do gerador).
    All,
}

/// Como a troca carrega a raiz de um espaço.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlbTag {
    /// Sem PCID/ASID: escrever uma raiz diferente esvazia o TLB, como antes da 4e.
    Untagged,
    /// Com a etiqueta `index` (zero é a do kernel, que nunca sai do gerador).
    Tagged { index: u32, flush: TagFlush },
}
