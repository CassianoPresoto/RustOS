//! Gerador de PCID e ASID com contador de geração: o esquema do SDD 17 para várias CPUs.
//! Esgotou, esvazia tudo e começa outra geração. Puro: quem muda o `CR3` e o `TTBR0` é o `arch/`.

use core::num::NonZeroU64;

/// Um identificador de espaço de endereços: `geração << bits | índice`. A geração começa
/// em 1, então o valor nunca é zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Asid(NonZeroU64);

impl Asid {
    /// O valor inteiro (geração e índice), para guardar no espaço de endereços.
    pub const fn raw(self) -> u64 {
        self.0.get()
    }
}

/// Por que o gerador recusou o pedido.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsidError {
    /// Largura zero, acima de 32 bits ou maior que o bitmap comporta.
    BadWidth,
    /// Sobram, fora os reservados, no máximo `CPUS` índices: um esgotamento não acharia lugar.
    TooFewIds,
    /// A geração não cabe mais acima dos bits do índice.
    GenerationOverflow,
    /// `cpu` fora de `0..CPUS`.
    BadCpu,
}

/// O que `switch` manda a CPU fazer ao trocar de espaço.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Switch {
    /// O valor que o espaço passa a ter; a CPU o carrega no `CR3` ou no `TTBR0`.
    pub asid: Asid,
    /// Esvaziar o TLB desta CPU **antes** de carregar `asid` (a regra de geração).
    pub flush: bool,
}

/// O gerador global, com `WORDS * 64` índices no máximo e `CPUS` CPUs. Nenhum índice volta
/// ao uso antes de a geração subir: quem sai do sistema só devolve o índice no esgotamento.
/// Toda troca passa por `&mut self`, logo pelo lock do chamador: DEBT-0076.
#[derive(Debug)]
pub struct AsidAllocator<const WORDS: usize, const CPUS: usize> {
    bits: u32,
    first: u32,
    generation: u64,
    cursor: u32,
    used: [u64; WORDS],
    active: [Option<Asid>; CPUS],
    reserved: [Option<Asid>; CPUS],
    flush_pending: [bool; CPUS],
}

impl<const WORDS: usize, const CPUS: usize> AsidAllocator<WORDS, CPUS> {
    /// `bits` é a largura do índice (PCID: 12; ASID: 8 ou 16); índices abaixo de `first`
    /// nunca saem (o PCID 0, o ASID da raiz vazia). Toda CPU esvazia na primeira troca.
    pub const fn new(bits: u32, first: u32) -> Result<Self, AsidError> {
        if bits == 0 || bits > 32 || (1_u64 << bits) > (WORDS as u64) * 64 {
            return Err(AsidError::BadWidth);
        }
        let ids = 1_u64 << bits;
        if (first as u64) >= ids || ids - (first as u64) <= CPUS as u64 {
            return Err(AsidError::TooFewIds);
        }
        Ok(Self {
            bits,
            first,
            generation: 1,
            cursor: first,
            used: [0; WORDS],
            active: [None; CPUS],
            reserved: [None; CPUS],
            flush_pending: [true; CPUS],
        })
    }

    /// A geração em curso (começa em 1).
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// A largura do índice, em bits.
    pub const fn bits(&self) -> u32 {
        self.bits
    }

    /// Índice de `asid`: o que vai nos 12 bits do `CR3` ou nos bits 63:48 do `TTBR0`.
    pub const fn index(&self, asid: Asid) -> u32 {
        u32_of(asid.raw() & ((1_u64 << self.bits) - 1))
    }

    /// A geração em que `asid` foi dado.
    pub const fn generation_of(&self, asid: Asid) -> u64 {
        asid.raw() >> self.bits
    }

    /// `asid` é da geração em curso.
    pub const fn is_current(&self, asid: Asid) -> bool {
        self.generation_of(asid) == self.generation
    }

    /// Quantos índices estão tomados na geração em curso.
    pub fn in_use(&self) -> u32 {
        self.used.iter().map(|word| word.count_ones()).sum()
    }

    /// A CPU `cpu` vai rodar o espaço que tem `current` (`None` se ainda não tem id).
    /// Quem guarda o valor devolvido é o espaço; o gerador guarda só o que a CPU roda.
    pub fn switch(&mut self, cpu: usize, current: Option<Asid>) -> Result<Switch, AsidError> {
        if cpu >= CPUS {
            return Err(AsidError::BadCpu);
        }
        let asid = match current {
            Some(asid) if self.is_current(asid) => asid,
            _ => self.new_context(current)?,
        };
        let flush = core::mem::take(&mut self.flush_pending[cpu]);
        self.active[cpu] = Some(asid);
        Ok(Switch { asid, flush })
    }

    fn pack(&self, generation: u64, index: u32) -> Result<Asid, AsidError> {
        let high = generation
            .checked_mul(1_u64 << self.bits)
            .ok_or(AsidError::GenerationOverflow)?;
        NonZeroU64::new(high | u64::from(index))
            .map(Asid)
            .ok_or(AsidError::GenerationOverflow)
    }

    fn new_context(&mut self, old: Option<Asid>) -> Result<Asid, AsidError> {
        if let Some(old) = old {
            let index = self.index(old);
            let renewed = self.pack(self.generation, index)?;
            if self.renew_reserved(old, renewed) || self.take(index) {
                return Ok(renewed);
            }
        }
        if let Some(index) = self.find_free(self.cursor) {
            self.cursor = index;
            return self.claim(index);
        }
        self.rollover()?;
        let index = self.find_free(self.first).ok_or(AsidError::TooFewIds)?;
        self.cursor = index;
        self.claim(index)
    }

    fn claim(&mut self, index: u32) -> Result<Asid, AsidError> {
        let asid = self.pack(self.generation, index)?;
        self.take(index);
        Ok(asid)
    }

    /// Passa o valor reservado de qualquer CPU de `old` para `renewed`; `true` se achou.
    fn renew_reserved(&mut self, old: Asid, renewed: Asid) -> bool {
        let mut hit = false;
        for slot in &mut self.reserved {
            if *slot == Some(old) {
                *slot = Some(renewed);
                hit = true;
            }
        }
        hit
    }

    /// Esgotou: sobe a geração, esvazia o bitmap e guarda só o que cada CPU roda agora.
    fn rollover(&mut self) -> Result<(), AsidError> {
        let next = self.generation + 1;
        self.pack(next, 0)?;
        self.generation = next;
        self.used = [0; WORDS];
        for cpu in 0..CPUS {
            let running = self.active[cpu].take().or(self.reserved[cpu]);
            if let Some(asid) = running {
                self.take(self.index(asid));
            }
            self.reserved[cpu] = running;
            self.flush_pending[cpu] = true;
        }
        Ok(())
    }

    /// Marca `index` como tomado; `true` se estava livre.
    fn take(&mut self, index: u32) -> bool {
        let word = &mut self.used[(index / 64) as usize];
        let bit = 1_u64 << (index % 64);
        let was_free = *word & bit == 0;
        *word |= bit;
        was_free
    }

    fn find_free(&self, from: u32) -> Option<u32> {
        let end = 1_u32.checked_shl(self.bits).unwrap_or(u32::MAX);
        (from..end).find(|&index| self.used[(index / 64) as usize] >> (index % 64) & 1 == 0)
    }
}

#[allow(
    clippy::cast_possible_truncation,
    reason = "a máscara tem no máximo 32 bits"
)]
const fn u32_of(value: u64) -> u32 {
    value as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::XorShift;

    type Small = AsidAllocator<1, 2>;

    fn switched(alloc: &mut Small, cpu: usize, current: Option<Asid>) -> Switch {
        alloc.switch(cpu, current).unwrap()
    }

    #[test]
    fn new_validates_the_width_and_the_headroom() {
        assert_eq!(Small::new(0, 0).unwrap_err(), AsidError::BadWidth);
        assert_eq!(Small::new(7, 0).unwrap_err(), AsidError::BadWidth);
        assert_eq!(Small::new(33, 0).unwrap_err(), AsidError::BadWidth);
        assert!(Small::new(6, 0).is_ok());
        assert!(Small::new(6, 61).is_ok());
        assert_eq!(Small::new(6, 62).unwrap_err(), AsidError::TooFewIds);
        assert_eq!(Small::new(6, 64).unwrap_err(), AsidError::TooFewIds);
        assert_eq!(Small::new(1, 0).unwrap_err(), AsidError::TooFewIds);
        assert!(AsidAllocator::<4, 3>::new(8, 0).is_ok());
        assert_eq!(
            AsidAllocator::<4, 3>::new(9, 0).unwrap_err(),
            AsidError::BadWidth
        );
    }

    #[test]
    fn real_widths_fit_their_bitmaps() {
        assert!(AsidAllocator::<4, 8>::new(8, 1).is_ok());
        assert!(AsidAllocator::<64, 8>::new(12, 1).is_ok());
        assert!(AsidAllocator::<1024, 8>::new(16, 1).is_ok());
        assert_eq!(
            AsidAllocator::<64, 8>::new(16, 1).unwrap_err(),
            AsidError::BadWidth
        );
        assert_eq!(size_of::<AsidAllocator<64, 8>>(), 672);
        assert_eq!(size_of::<AsidAllocator<1024, 8>>(), 8352);
    }

    #[test]
    fn indices_come_in_order_from_the_first_allowed() {
        let mut alloc = AsidAllocator::<1, 2>::new(6, 2).unwrap();
        let ids: Vec<u32> = (0..5)
            .map(|_| {
                let asid = alloc.switch(0, None).unwrap().asid;
                alloc.index(asid)
            })
            .collect();
        assert_eq!(ids, [2, 3, 4, 5, 6]);
        assert_eq!(alloc.generation(), 1);
        assert_eq!(alloc.in_use(), 5);
    }

    #[test]
    fn the_first_switch_of_each_cpu_flushes_and_the_next_does_not() {
        let mut alloc = Small::new(6, 1).unwrap();
        let a = switched(&mut alloc, 0, None);
        assert!(a.flush);
        assert!(!switched(&mut alloc, 0, Some(a.asid)).flush);
        assert!(switched(&mut alloc, 1, Some(a.asid)).flush);
        assert!(!switched(&mut alloc, 1, Some(a.asid)).flush);
    }

    #[test]
    fn current_generation_value_is_returned_unchanged() {
        let mut alloc = Small::new(6, 1).unwrap();
        let a = switched(&mut alloc, 0, None).asid;
        assert!(alloc.is_current(a));
        assert_eq!(alloc.generation_of(a), 1);
        assert_eq!(switched(&mut alloc, 1, Some(a)).asid, a);
        assert_eq!(alloc.in_use(), 1);
    }

    #[test]
    fn exhaustion_starts_a_new_generation_and_flushes_every_cpu() {
        let mut alloc = AsidAllocator::<1, 2>::new(3, 1).unwrap();
        let mut spaces: Vec<Option<Asid>> = vec![None; 8];
        for space in spaces.iter_mut().take(7) {
            *space = Some(alloc.switch(0, *space).unwrap().asid);
        }
        assert_eq!((alloc.generation(), alloc.in_use()), (1, 7));
        assert!(!alloc.switch(0, spaces[0]).unwrap().flush);
        let fresh = alloc.switch(0, None).unwrap();
        assert!(fresh.flush);
        assert_eq!(alloc.generation(), 2);
        assert_eq!(alloc.generation_of(fresh.asid), 2);
        assert_eq!(alloc.in_use(), 2);
        assert!(alloc.switch(1, spaces[1]).unwrap().flush);
    }

    #[test]
    fn a_space_running_at_the_rollover_keeps_its_index_in_the_new_generation() {
        let mut alloc = AsidAllocator::<1, 2>::new(3, 1).unwrap();
        let mut spaces: Vec<Option<Asid>> = vec![None; 8];
        for space in spaces.iter_mut().take(7) {
            *space = Some(alloc.switch(0, *space).unwrap().asid);
        }
        let running = spaces[6].unwrap();
        alloc.switch(0, None).unwrap();
        let renewed = alloc.switch(0, Some(running)).unwrap().asid;
        assert_ne!(renewed, running);
        assert_eq!(alloc.index(renewed), alloc.index(running));
        assert!(alloc.is_current(renewed));
    }

    #[test]
    fn a_space_from_an_old_generation_gets_a_current_value() {
        let mut alloc = AsidAllocator::<1, 2>::new(3, 1).unwrap();
        let mut spaces: Vec<Option<Asid>> = vec![None; 3];
        for space in &mut spaces {
            *space = Some(alloc.switch(0, *space).unwrap().asid);
        }
        let old = spaces[0].unwrap();
        for _ in 0..5 {
            alloc.switch(0, None).unwrap();
        }
        assert_eq!(alloc.generation(), 2);
        let used = alloc.in_use();
        let renewed = alloc.switch(1, Some(old)).unwrap().asid;
        assert_ne!(renewed, old);
        assert!(alloc.is_current(renewed));
        assert!(alloc.in_use() <= used + 1);
    }

    #[test]
    fn two_cpus_running_one_space_both_keep_it_across_a_rollover() {
        let mut alloc = AsidAllocator::<1, 3>::new(3, 1).unwrap();
        let shared = alloc.switch(0, None).unwrap().asid;
        alloc.switch(1, Some(shared)).unwrap();
        for _ in 0..7 {
            alloc.switch(2, None).unwrap();
        }
        assert_eq!(alloc.generation(), 2);
        let renewed = alloc.switch(0, Some(shared)).unwrap().asid;
        assert_eq!(alloc.reserved[..2], [Some(renewed), Some(renewed)]);
        assert_eq!(alloc.switch(1, Some(renewed)).unwrap().asid, renewed);
        assert_eq!(alloc.index(renewed), alloc.index(shared));
    }

    #[test]
    fn double_rollover_idle_cpu_keeps_index() {
        let mut alloc = AsidAllocator::<1, 3>::new(3, 1).unwrap();
        let idle = alloc.switch(0, None).unwrap().asid;
        for _ in 0..40 {
            if alloc.generation() == 3 {
                break;
            }
            alloc.switch(2, None).unwrap();
        }
        assert_eq!(alloc.generation(), 3);
        assert_eq!(alloc.reserved[0], Some(idle));
        let renewed = alloc.switch(1, Some(idle)).unwrap().asid;
        assert!(alloc.is_current(renewed));
        assert_eq!(alloc.index(renewed), alloc.index(idle));
        assert_ne!(renewed, idle);
    }

    #[test]
    fn a_dead_space_gives_nothing_back_before_the_rollover() {
        let mut alloc = AsidAllocator::<1, 2>::new(3, 1).unwrap();
        for _ in 0..7 {
            alloc.switch(0, None).unwrap();
        }
        assert_eq!((alloc.generation(), alloc.in_use()), (1, 7));
        alloc.switch(0, None).unwrap();
        assert_eq!(alloc.generation(), 2);
    }

    #[test]
    fn bad_cpu_is_refused_without_touching_the_state() {
        let mut alloc = Small::new(6, 1).unwrap();
        assert_eq!(alloc.switch(2, None).unwrap_err(), AsidError::BadCpu);
        assert_eq!(alloc.in_use(), 0);
    }

    #[test]
    fn generation_overflow_is_an_error_and_changes_nothing() {
        let mut alloc = AsidAllocator::<1, 2>::new(3, 1).unwrap();
        alloc.generation = u64::MAX >> 3;
        for _ in 0..7 {
            alloc.switch(0, None).unwrap();
        }
        let used = alloc.in_use();
        assert_eq!(
            alloc.switch(0, None).unwrap_err(),
            AsidError::GenerationOverflow
        );
        assert_eq!((alloc.generation(), alloc.in_use()), (u64::MAX >> 3, used));
    }

    #[test]
    fn generation_that_does_not_fit_above_the_index_is_refused_at_the_first_use() {
        let mut alloc = AsidAllocator::<1, 2>::new(3, 1).unwrap();
        alloc.generation = (u64::MAX >> 3) + 1;
        assert_eq!(
            alloc.switch(0, None).unwrap_err(),
            AsidError::GenerationOverflow
        );
    }

    /// Um espaço vivo e o que cada CPU guarda no TLB, rotulado por índice.
    struct Sim<const CPUS: usize> {
        tlb: [Vec<Option<usize>>; CPUS],
        honour_flush: bool,
        violations: u32,
    }

    impl<const CPUS: usize> Sim<CPUS> {
        fn new(ids: usize, honour_flush: bool) -> Self {
            Self {
                tlb: core::array::from_fn(|_| vec![None; ids]),
                honour_flush,
                violations: 0,
            }
        }

        fn load(&mut self, cpu: usize, index: usize, space: usize, flush: bool) {
            if flush && self.honour_flush {
                self.tlb[cpu].fill(None);
            }
            match self.tlb[cpu][index] {
                Some(owner) if owner != space => self.violations += 1,
                _ => {}
            }
            self.tlb[cpu][index] = Some(space);
        }
    }

    const SPACES: usize = 40;
    const CPU_COUNT: usize = 4;

    fn fuzz(seed: u64, bits: u32, steps: u32, honour_flush: bool) -> (u32, u64) {
        let mut alloc = AsidAllocator::<1, CPU_COUNT>::new(bits, 1).unwrap();
        let ids = 1_usize << bits;
        let mut sim = Sim::<CPU_COUNT>::new(ids, honour_flush);
        let mut rng = XorShift::new(seed);
        let mut asids: [Option<Asid>; SPACES] = [None; SPACES];
        let mut identity: [usize; SPACES] = core::array::from_fn(|i| i);
        let mut next_identity = SPACES;
        let mut last_generation = alloc.generation();
        let mut running: [Option<usize>; CPU_COUNT] = [None; CPU_COUNT];
        let mut loaded = [0_u32; CPU_COUNT];
        for step in 0..steps {
            let space = rng.below(SPACES);
            if rng.below(12) == 0 {
                for slot in &mut running {
                    if *slot == Some(space) {
                        *slot = None;
                    }
                }
                asids[space] = None;
                identity[space] = next_identity;
                next_identity += 1;
                continue;
            }
            // A CPU 0 quase não troca: fica rodando o mesmo espaço por vários esgotamentos.
            let cpu = if rng.below(64) == 0 {
                0
            } else {
                1 + rng.below(CPU_COUNT - 1)
            };
            let before = asids[space];
            let got = alloc.switch(cpu, before).unwrap();
            asids[space] = Some(got.asid);
            let ctx = format!("semente {seed:#x}, passo {step}");
            assert!(alloc.is_current(got.asid), "{ctx}");
            assert!(alloc.index(got.asid) >= 1, "{ctx}");
            if let Some(stale) = before.filter(|&old| !alloc.is_current(old)) {
                assert_ne!(stale, got.asid, "{ctx}: id velho devolvido");
            }
            sim.load(
                cpu,
                alloc.index(got.asid) as usize,
                identity[space],
                got.flush,
            );
            let generation = alloc.generation();
            assert!(
                generation == last_generation || generation == last_generation + 1,
                "{ctx}"
            );
            if generation != last_generation {
                assert!(
                    alloc.in_use() <= u32::try_from(CPU_COUNT).unwrap() + 1,
                    "{ctx}: bitmap não esvaziou"
                );
                assert!(got.flush, "{ctx}: a CPU da virada tem de esvaziar");
            }
            last_generation = generation;
            running[cpu] = Some(space);
            loaded[cpu] = alloc.index(got.asid);
            for (other, slot) in running.iter().enumerate() {
                let Some(owner) = *slot else { continue };
                let current = asids[owner].expect("espaço rodando sem valor");
                assert_eq!(
                    alloc.index(current),
                    loaded[other],
                    "{ctx}: a CPU {other} roda o espaço {owner} com outro índice"
                );
            }
            let mut taken = [false; 1 << 6];
            for asid in asids.iter().flatten().filter(|&&a| alloc.is_current(a)) {
                let index = alloc.index(*asid) as usize;
                assert!(!taken[index], "{ctx}: índice {index} em dois espaços");
                taken[index] = true;
            }
        }
        (sim.violations, last_generation)
    }

    #[test]
    fn random_switches_never_share_an_id_nor_leave_a_stale_tlb_entry() {
        for (seed, bits) in [(1, 4), (2, 5), (3, 4), (0xFEED, 6), (9, 5)] {
            let (violations, generation) = fuzz(seed, bits, 20_000, true);
            assert_eq!(violations, 0, "semente {seed:#x}");
            assert!(
                generation > 10,
                "o teste precisa passar por várias gerações"
            );
        }
    }

    #[test]
    fn the_simulation_catches_a_cpu_that_skips_the_flush() {
        let (violations, _) = fuzz(1, 4, 20_000, false);
        assert!(violations > 0);
    }
}
