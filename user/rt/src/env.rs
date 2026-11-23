//! O que o kernel deixa na pilha inicial: `argc`, `argv`, `envp` e o auxv (SDD 17). O programa
//! lê tudo por aqui, em valores, sem ponteiro.

use core::marker::PhantomData;
use core::{ptr, slice};

use rustos_hal::abi::{ARGS_BYTES_MAX, ARGS_MAX, AT_RANDOM_LEN, auxv};

/// Palavras que a leitura percorre antes de desistir. É só um teto de sanidade: o `rt` não sabe
/// onde a pilha acaba e confia que o kernel a montou com os zeros de `argv`, `envp` e o `AT_NULL`.
const WALK_LIMIT: usize = 4096;
/// `PT_LOAD`.
const PT_LOAD: u32 = 1;
/// Tamanho de um program header ELF64.
const PHDR_LEN: usize = 56;

/// As entradas do auxv que a fase 4 define; a que falta fica em zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Auxv {
    pub phdr: u64,
    pub phent: u64,
    pub phnum: u64,
    pub pagesz: u64,
    pub entry: u64,
    pub hwcap: u64,
    pub hwcap2: u64,
    /// `AT_RANDOM` existe e os 16 bytes estão em `Env::random`.
    pub has_random: bool,
}

/// Os argumentos do programa: o `argv` inteiro, com o nome primeiro. Cada item são os bytes da cadeia
/// que o kernel pôs na pilha, sem o `0`; o texto é do programa, o kernel não diz a codificação.
#[derive(Clone, Debug)]
pub struct Args<'a> {
    pointers: u64,
    count: usize,
    index: usize,
    _life: PhantomData<&'a [u8]>,
}

impl<'a> Iterator for Args<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<&'a [u8]> {
        if self.index >= self.count {
            return None;
        }
        let slot = self.pointers + 8 * self.index as u64;
        self.index += 1;
        // SAFETY: `slot` é o `argv[index]` que `Env::from_stack` deixou contado em `argc`, e
        // `count` não passa de `argc`; a cadeia dele vive enquanto o processo existe.
        Some(unsafe { string_at(slot) })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.count - self.index;
        (left, Some(left))
    }
}

impl ExactSizeIterator for Args<'_> {}

/// O ambiente do programa: o que o `_start` recebeu. Só `from_stack` o preenche e os campos são
/// privados, para o programa em Rust seguro não forjar um ponteiro que o `rt` leria.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Env {
    boot_handle: u64,
    initial_sp: u64,
    argc: u64,
    /// Endereço do `argv[0]` na pilha (a palavra depois de `argc`).
    argv: u64,
    auxv: Auxv,
    random: [u8; AT_RANDOM_LEN],
    loads: Option<usize>,
}

impl Env {
    /// Lê `argc`, pula `argv` e `envp` e junta o auxv.
    ///
    /// # Safety
    /// `stack` é a pilha inicial que o kernel monta (`abi::InitialStack`), com o fim de cada vetor.
    pub unsafe fn from_stack(stack: *const u64, boot_handle: u64) -> Self {
        let mut env = Self {
            boot_handle,
            initial_sp: stack.addr() as u64,
            argv: stack.addr() as u64 + 8,
            ..Self::default()
        };
        // SAFETY: o contrato da função; `argc` é a primeira palavra.
        env.argc = unsafe { stack.read() };
        let mut index = 1;
        for _ in 0..2 {
            loop {
                // SAFETY: o contrato da função; o laço pára no zero que fecha `argv` e `envp`.
                let word = unsafe { stack.wrapping_add(index).read() };
                index += 1;
                if word == 0 || index > WALK_LIMIT {
                    break;
                }
            }
        }
        while index < WALK_LIMIT {
            // SAFETY: o contrato da função; o auxv são pares até `AT_NULL`.
            let tag = unsafe { stack.wrapping_add(index).read() };
            // SAFETY: o mesmo par: o valor vem logo depois da tag.
            let value = unsafe { stack.wrapping_add(index + 1).read() };
            index += 2;
            match tag {
                auxv::AT_NULL => break,
                auxv::AT_PHDR => env.auxv.phdr = value,
                auxv::AT_PHENT => env.auxv.phent = value,
                auxv::AT_PHNUM => env.auxv.phnum = value,
                auxv::AT_PAGESZ => env.auxv.pagesz = value,
                auxv::AT_ENTRY => env.auxv.entry = value,
                auxv::AT_HWCAP => env.auxv.hwcap = value,
                auxv::AT_HWCAP2 => env.auxv.hwcap2 = value,
                auxv::AT_RANDOM => env.read_random(value),
                _ => {}
            }
        }
        // SAFETY: o contrato da função: se o auxv traz `AT_PHDR`, a tabela inteira está num `PT_LOAD`.
        env.loads = unsafe { env.count_loads() };
        env
    }

    /// O valor de `rdi`/`x0` na entrada: a ponta do canal de boot, ou `0` sem ela.
    pub const fn boot_handle(&self) -> u64 {
        self.boot_handle
    }

    /// A ponta do canal de boot, se o programa nasceu com uma.
    pub const fn boot_channel(&self) -> Option<rustos_hal::abi::Handle> {
        rustos_hal::abi::Handle::from_raw(self.boot_handle)
    }

    /// O ponteiro de pilha da entrada.
    pub const fn initial_sp(&self) -> u64 {
        self.initial_sp
    }

    pub const fn argc(&self) -> u64 {
        self.argc
    }

    /// Os argumentos, com o nome do programa primeiro. Sem `argc`, nenhum.
    pub fn args(&self) -> Args<'_> {
        Args {
            pointers: self.argv,
            count: usize::try_from(self.argc).unwrap_or(ARGS_MAX).min(ARGS_MAX),
            index: 0,
            _life: PhantomData,
        }
    }

    pub const fn auxv(&self) -> &Auxv {
        &self.auxv
    }

    /// Os 16 bytes de `AT_RANDOM`, copiados da pilha.
    pub const fn random(&self) -> &[u8; AT_RANDOM_LEN] {
        &self.random
    }

    /// Quantos `PT_LOAD` a tabela de program headers do `AT_PHDR` tem; `None` sem `AT_PHDR` ou com
    /// `AT_PHENT` que não é o de um ELF64.
    pub const fn load_segments(&self) -> Option<usize> {
        self.loads
    }

    #[allow(
        clippy::cast_possible_truncation,
        reason = "o usuário só existe em 64 bits"
    )]
    fn read_random(&mut self, address: u64) {
        let source = ptr::with_exposed_provenance::<u8>(address as usize);
        // SAFETY: o kernel aponta `AT_RANDOM` para 16 bytes legíveis no topo da pilha.
        unsafe { ptr::copy_nonoverlapping(source, self.random.as_mut_ptr(), AT_RANDOM_LEN) };
        self.auxv.has_random = true;
    }

    /// # Safety
    /// Se `AT_PHDR` existe, a tabela de `AT_PHNUM` entradas de `AT_PHENT` bytes está mapeada.
    unsafe fn count_loads(&self) -> Option<usize> {
        if self.auxv.phdr == 0 || self.auxv.phent != PHDR_LEN as u64 {
            return None;
        }
        let table = ptr::with_exposed_provenance::<u8>(usize::try_from(self.auxv.phdr).ok()?);
        let count = usize::try_from(self.auxv.phnum).ok()?;
        let loads = (0..count).filter(|&index| {
            let entry = table.wrapping_add(index * PHDR_LEN).cast::<[u8; 4]>();
            // SAFETY: o contrato da função; o campo `p_type` abre cada entrada.
            let raw = unsafe { entry.read_unaligned() };
            u32::from_le_bytes(raw) == PT_LOAD
        });
        Some(loads.count())
    }
}

/// Lê a cadeia que `slot` aponta, até o `0`.
///
/// # Safety
/// `slot` é um `argv[i]` da pilha: a cadeia tem `0` antes de `ARGS_BYTES_MAX` bytes.
#[allow(
    clippy::cast_possible_truncation,
    reason = "o usuário só existe em 64 bits"
)]
unsafe fn string_at<'a>(slot: u64) -> &'a [u8] {
    let address = slot as usize;
    // SAFETY: o contrato da função: `slot` é uma palavra da pilha, alinhada a 8.
    let word = unsafe { ptr::with_exposed_provenance::<u64>(address).read() };
    let start = ptr::with_exposed_provenance::<u8>(word as usize);
    let mut len = 0;
    while len < ARGS_BYTES_MAX {
        // SAFETY: o contrato da função: a cadeia tem um `0` antes de `ARGS_BYTES_MAX` bytes.
        let byte = unsafe { start.wrapping_add(len).read() };
        if byte == 0 {
            break;
        }
        len += 1;
    }
    // SAFETY: os `len` bytes de `start` são a cadeia que o laço acima acabou de ler.
    unsafe { slice::from_raw_parts(start, len) }
}

#[cfg(test)]
mod tests {
    use rustos_hal::abi::{InitialStack, StartInfo};

    use super::*;

    /// Uma pilha de mentira: os bytes de `InitialStack` copiados para um vetor de palavras, com os
    /// ponteiros do auxv e de `argv` refeitos para o próprio vetor.
    fn fake_stack(
        phdr: Option<u64>,
        random: [u8; AT_RANDOM_LEN],
        args: &[&[u8]],
    ) -> (Vec<u64>, usize) {
        let words = 64;
        let mut memory = vec![0_u64; words];
        let top = memory.as_ptr().addr() as u64 + (words as u64) * 8;
        let top = top & !15;
        let info = StartInfo {
            entry: 0x40_1000,
            phdr,
            phent: 56,
            phnum: 3,
            random,
        };
        let stack = InitialStack::build(top, &info, args).unwrap();
        let start = usize::try_from(stack.sp() - memory.as_ptr().addr() as u64).unwrap() / 8;
        for (index, chunk) in stack.bytes().as_chunks::<8>().0.iter().enumerate() {
            memory[start + index] = u64::from_le_bytes(*chunk);
        }
        (memory, start)
    }

    #[test]
    fn it_reads_the_stack_the_kernel_builds() {
        let random = *b"fedcba9876543210";
        let mut table = [0_u8; 3 * PHDR_LEN];
        table[0..4].copy_from_slice(&6_u32.to_le_bytes());
        table[PHDR_LEN..PHDR_LEN + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
        table[2 * PHDR_LEN..2 * PHDR_LEN + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
        let (memory, start) = fake_stack(Some(table.as_ptr().addr() as u64), random, &[]);
        // SAFETY: o vetor tem a forma de uma pilha inicial; `AT_RANDOM` e `AT_PHDR` apontam para
        // memória viva do teste.
        let env = unsafe { Env::from_stack(memory[start..].as_ptr(), 0) };
        assert_eq!((env.argc(), env.boot_handle()), (0, 0));
        let aux = env.auxv();
        assert_eq!((aux.pagesz, aux.entry), (4096, 0x40_1000));
        assert_eq!((aux.phent, aux.phnum), (56, 3));
        assert_eq!((aux.hwcap, aux.hwcap2), (0, 0));
        assert!(aux.has_random);
        assert_eq!(env.random(), &random);
        assert_eq!(env.initial_sp() % 16, 0);
        assert_eq!(env.load_segments(), Some(2));
    }

    #[test]
    fn a_stack_without_phdr_has_no_load_count() {
        let (memory, start) = fake_stack(None, [7; AT_RANDOM_LEN], &[]);
        // SAFETY: o mesmo vetor de pilha inicial.
        let env = unsafe { Env::from_stack(memory[start..].as_ptr(), 9) };
        assert_eq!((env.auxv().phdr, env.auxv().phnum), (0, 0));
        assert_eq!(env.boot_handle(), 9);
        assert_eq!(env.load_segments(), None);
    }

    #[test]
    fn a_phent_that_is_not_an_elf64_entry_gives_no_load_count() {
        let memory = [
            0,
            0,
            0,
            auxv::AT_PHDR,
            0x1000,
            auxv::AT_PHENT,
            32,
            auxv::AT_NULL,
            0,
        ];
        // SAFETY: o vetor tem a forma de uma pilha inicial; `AT_PHENT` errado impede a leitura da tabela.
        let env = unsafe { Env::from_stack(memory.as_ptr(), 0) };
        assert_eq!(env.load_segments(), None);
    }

    #[test]
    fn argv_and_envp_are_skipped_before_the_auxv() {
        // argc 1, argv[0] = 0x1234, NULL, envp[0] = 0x5678, NULL, AT_PAGESZ, AT_NULL.
        let memory = [
            1,
            0x1234,
            0,
            0x5678,
            0,
            auxv::AT_PAGESZ,
            4096,
            auxv::AT_NULL,
            0,
        ];
        // SAFETY: o vetor tem a forma de uma pilha inicial, sem `AT_RANDOM`.
        let env = unsafe { Env::from_stack(memory.as_ptr(), 0) };
        assert_eq!((env.argc(), env.auxv().pagesz), (1, 4096));
        assert!(!env.auxv().has_random);
    }

    #[test]
    fn unknown_tags_are_ignored() {
        let memory = [
            0,
            0,
            0,
            0x7777,
            5,
            auxv::AT_ENTRY,
            0x40_2000,
            auxv::AT_NULL,
            0,
        ];
        // SAFETY: o vetor tem a forma de uma pilha inicial, sem `AT_RANDOM`.
        let env = unsafe { Env::from_stack(memory.as_ptr(), 0) };
        assert_eq!(env.auxv().entry, 0x40_2000);
    }

    #[test]
    fn the_walk_gives_up_at_the_limit_on_a_stack_without_end_marks() {
        let memory = vec![1_u64; WALK_LIMIT + 16];
        // SAFETY: o vetor é maior que o limite da leitura, que pára nele.
        let env = unsafe { Env::from_stack(memory.as_ptr(), 0) };
        assert_eq!(env.auxv(), &Auxv::default());
    }

    #[test]
    fn args_are_the_strings_the_kernel_put_on_the_stack() {
        let (memory, start) = fake_stack(None, [1; AT_RANDOM_LEN], &[b"count", b"3"]);
        // SAFETY: o vetor tem a forma de uma pilha inicial, com `argv` nos endereços reais.
        let env = unsafe { Env::from_stack(memory[start..].as_ptr(), 0) };
        assert_eq!(env.argc(), 2);
        let args: Vec<&[u8]> = env.args().collect();
        assert_eq!(args, [&b"count"[..], &b"3"[..]]);
        assert_eq!(env.args().len(), 2);
    }

    #[test]
    fn a_hand_made_stack_with_argc_one_gives_the_program_name() {
        let name = *b"count\0";
        // argc 1, argv[0], o zero que fecha argv, o zero de envp, AT_NULL.
        let memory = [1, name.as_ptr().addr() as u64, 0, 0, auxv::AT_NULL, 0];
        // SAFETY: o vetor tem a forma de uma pilha inicial; `argv[0]` aponta para `name`, que vive aqui.
        let env = unsafe { Env::from_stack(memory.as_ptr(), 0) };
        let mut args = env.args();
        assert_eq!(args.next(), Some(&b"count"[..]));
        assert_eq!(args.next(), None);
    }

    #[test]
    fn a_stack_without_argc_has_no_arguments() {
        let memory = [0, 0, 0, auxv::AT_NULL, 0];
        // SAFETY: o vetor tem a forma de uma pilha inicial sem argumentos.
        let env = unsafe { Env::from_stack(memory.as_ptr(), 0) };
        assert_eq!(env.args().count(), 0);
    }
}
