//! Classe justa: EEVDF do Linux 6.6+ em tamanho pequeno (SDD 14). Fórmulas e
//! fontes no capítulo; aqui só o que quebra se mudar a aritmética.

use crate::TaskId;
use crate::scheduler::{Detached, SchedError, SchedParams, Scheduler};
use crate::table::{IdTable, NIL, slot_u16};
use crate::tree::{DeadlineTree, signed_diff};
use crate::weight::{real_to_virtual, virtual_to_real, weight_of_nice};

/// `sysctl_sched_base_slice` do Linux: 0,70 ms.
pub const BASE_SLICE_NS: u64 = 700_000;
/// Limites do pedido de fatia por tarefa em `fair.c` (`sched_runtime`): 0,1 ms a 100 ms.
pub const MIN_SLICE_NS: u64 = 100_000;
pub const MAX_SLICE_NS: u64 = 100_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EevdfConfig {
    /// Fatia de quem não pediu uma (`request size` do EEVDF).
    pub base_slice_ns: u64,
}

impl Default for EevdfConfig {
    fn default() -> Self {
        Self {
            base_slice_ns: BASE_SLICE_NS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Sleeping,
    Ready,
    Running,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Entity {
    vruntime: u64,
    deadline: u64,
    vprot: u64,
    slice_ns: u64,
    vlag: i64,
    sleep_vtime: u64,
    weight: u32,
    nice: i8,
    state: State,
}

const FRESH: Entity = Entity {
    vruntime: 0,
    deadline: 0,
    vprot: 0,
    slice_ns: BASE_SLICE_NS,
    vlag: 0,
    sleep_vtime: 0,
    weight: 1024,
    nice: 0,
    state: State::Sleeping,
};

fn saturate_i64(value: i128) -> i64 {
    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
}

#[derive(Debug)]
pub struct Eevdf<const N: usize> {
    config: EevdfConfig,
    table: IdTable<N>,
    ents: [Entity; N],
    tree: DeadlineTree<N>,
    curr: u16,
    zero_vruntime: u64,
    sum_weight: i64,
    sum_w_vruntime: i128,
    runnable: usize,
    curr_start_ns: u64,
    max_slice_ns: u64,
    resched: bool,
}

impl<const N: usize> Eevdf<N> {
    pub const fn new() -> Self {
        Self::with_config(EevdfConfig {
            base_slice_ns: BASE_SLICE_NS,
        })
    }

    pub const fn with_config(config: EevdfConfig) -> Self {
        Self {
            config,
            table: IdTable::new(),
            ents: [FRESH; N],
            tree: DeadlineTree::new(),
            curr: NIL,
            zero_vruntime: 0,
            sum_weight: 0,
            sum_w_vruntime: 0,
            runnable: 0,
            curr_start_ns: 0,
            max_slice_ns: config.base_slice_ns,
            resched: false,
        }
    }

    pub fn contains(&self, task: TaskId) -> bool {
        self.table.find(task).is_some()
    }

    /// Tarefas prontas ou correndo.
    pub const fn runnable(&self) -> usize {
        self.runnable
    }

    pub fn has_runnable(&self) -> bool {
        self.runnable > 0
    }

    fn ent(&self, slot: u16) -> &Entity {
        &self.ents[usize::from(slot)]
    }

    fn ent_mut(&mut self, slot: u16) -> &mut Entity {
        &mut self.ents[usize::from(slot)]
    }

    fn key(&self, vruntime: u64) -> i128 {
        i128::from(signed_diff(vruntime, self.zero_vruntime))
    }

    /// `lag >= 0`, sem dividir: `sum((v_i - v0) * w_i) >= (v - v0) * sum(w_i)`.
    fn eligible(&self, vruntime: u64) -> bool {
        self.sum_w_vruntime >= self.key(vruntime) * i128::from(self.sum_weight)
    }

    fn sum_add(&mut self, slot: u16) {
        let ent = *self.ent(slot);
        self.sum_w_vruntime += self.key(ent.vruntime) * i128::from(ent.weight);
        self.sum_weight += i64::from(ent.weight);
        self.runnable += 1;
    }

    fn sum_sub(&mut self, slot: u16) {
        let ent = *self.ent(slot);
        self.sum_w_vruntime -= self.key(ent.vruntime) * i128::from(ent.weight);
        self.sum_weight -= i64::from(ent.weight);
        self.runnable -= 1;
    }

    /// Leva `zero_vruntime` ao piso de `avg_vruntime`: as chaves ficam pequenas.
    fn recenter(&mut self) {
        if self.sum_weight <= 0 {
            return;
        }
        let weight = i128::from(self.sum_weight);
        let delta = match i64::try_from(self.sum_w_vruntime) {
            Ok(sum) => i128::from(sum.div_euclid(self.sum_weight)),
            Err(_) => self.sum_w_vruntime.div_euclid(weight),
        };
        self.sum_w_vruntime -= weight * delta;
        self.zero_vruntime = self
            .zero_vruntime
            .wrapping_add(saturate_i64(delta).cast_unsigned());
    }

    /// `avg_vruntime`: o tempo virtual V do sistema, piso da média ponderada.
    pub fn avg_vruntime(&mut self) -> u64 {
        self.recenter();
        self.zero_vruntime
    }

    fn lag_limit(&self, weight: u32) -> i64 {
        let real = self.max_slice_ns.saturating_mul(2);
        i64::try_from(real_to_virtual(real, weight)).unwrap_or(i64::MAX)
    }

    /// Guarda o lag de quem sai da disputa, preso a duas fatias (`entity_lag`).
    fn save_lag(&mut self, slot: u16) {
        let average = self.avg_vruntime();
        let ent = *self.ent(slot);
        let limit = self.lag_limit(ent.weight);
        let lag = signed_diff(average, ent.vruntime).clamp(-limit, limit);
        let ent = self.ent_mut(slot);
        ent.vlag = lag;
        ent.sleep_vtime = average;
    }

    /// Lag negativo se queima com o tempo virtual que passou dormindo, como se a tarefa
    /// tivesse ficado na fila (`DELAY_DEQUEUE` com `DELAY_ZERO`); lag positivo fica.
    fn decay_lag(&mut self, slot: u16) {
        let ent = *self.ent(slot);
        if ent.vlag >= 0 {
            return;
        }
        let elapsed = signed_diff(self.avg_vruntime(), ent.sleep_vtime).max(0);
        self.ent_mut(slot).vlag = ent.vlag.saturating_add(elapsed).min(0);
    }

    /// `place_entity`: a tarefa volta com o lag que saiu, compensado pelo peso que
    /// ela mesma soma à média (`vl = vl' * (W + w) / W`).
    fn place(&mut self, slot: u16, initial: bool) {
        let average = self.avg_vruntime();
        let ent = *self.ent(slot);
        let mut lag = i128::from(ent.vlag);
        if self.runnable > 0 && self.sum_weight > 0 && lag != 0 {
            let product = lag * i128::from(ent.weight);
            lag += match i64::try_from(product) {
                Ok(narrow) => i128::from(narrow / self.sum_weight),
                Err(_) => product / i128::from(self.sum_weight),
            };
        }
        let vruntime = average.wrapping_sub(saturate_i64(lag).cast_unsigned());
        let mut vslice = real_to_virtual(ent.slice_ns, ent.weight);
        if initial {
            vslice /= 2;
        }
        let ent = self.ent_mut(slot);
        ent.vruntime = vruntime;
        ent.deadline = vruntime.wrapping_add(vslice);
    }

    fn enqueue(&mut self, slot: u16) {
        self.ent_mut(slot).state = State::Ready;
        self.sum_add(slot);
        let ent = *self.ent(slot);
        self.tree
            .insert(usize::from(slot), ent.deadline, ent.vruntime);
    }

    /// `update_curr` e `update_deadline`: cobra o tempo corrido e, esgotada a fatia,
    /// renova o prazo.
    fn update_curr(&mut self, now_ns: u64) {
        if self.curr == NIL {
            return;
        }
        let delta = now_ns.saturating_sub(self.curr_start_ns);
        if delta == 0 {
            return;
        }
        self.curr_start_ns = now_ns;
        let slot = self.curr;
        let ent = *self.ent(slot);
        let charged = real_to_virtual(delta, ent.weight);
        self.sum_w_vruntime += i128::from(charged) * i128::from(ent.weight);
        self.ent_mut(slot).vruntime = ent.vruntime.wrapping_add(charged);
        self.refresh_deadline(slot);
    }

    fn refresh_deadline(&mut self, slot: u16) {
        let ent = *self.ent(slot);
        if signed_diff(ent.vruntime, ent.deadline) < 0 {
            return;
        }
        let vslice = real_to_virtual(ent.slice_ns, ent.weight);
        self.ent_mut(slot).deadline = ent.vruntime.wrapping_add(vslice);
        self.recenter();
        if !self.tree.is_empty() {
            self.resched = true;
        }
    }

    /// O corrente corre até esgotar a fatia ou cruzar o ponto de lag zero
    /// (`RUN_TO_PARITY`).
    fn protected(&self, slot: u16) -> bool {
        let ent = self.ent(slot);
        signed_diff(ent.vruntime, ent.vprot) < 0 && self.eligible(ent.vruntime)
    }

    /// `pick_eevdf`: o elegível de menor prazo, com o corrente na disputa.
    fn pick(&self, protect: bool) -> u16 {
        let mut curr = self.curr;
        if curr != NIL && !self.eligible(self.ent(curr).vruntime) {
            curr = NIL;
        }
        if curr != NIL && protect && self.protected(curr) {
            return curr;
        }
        let best = self.tree.first_eligible(|v| self.eligible(v));
        match (best, curr) {
            (None, _) => curr,
            (Some(best), NIL) => slot_u16(best),
            (Some(best), curr) => {
                let (c, b) = (self.ent(curr), self.ent(slot_u16(best)));
                if signed_diff(c.deadline, b.deadline) < 0 {
                    curr
                } else {
                    slot_u16(best)
                }
            }
        }
    }

    /// Devolve o corrente à disputa, sem escolher outro (usado pela classe de cima).
    pub fn put_prev(&mut self, now_ns: u64) {
        self.update_curr(now_ns);
        if self.curr == NIL {
            return;
        }
        let slot = self.curr;
        self.curr = NIL;
        self.ent_mut(slot).state = State::Ready;
        let ent = *self.ent(slot);
        self.tree
            .insert(usize::from(slot), ent.deadline, ent.vruntime);
    }

    /// Wakeup preemption: `RUN_TO_PARITY` protege o corrente, e `PREEMPT_SHORT` deixa
    /// uma fatia menor furar a proteção.
    fn check_preempt(&mut self, woken: u16) {
        if self.curr == NIL {
            self.resched = true;
            return;
        }
        let shorter = self.ent(woken).slice_ns < self.ent(self.curr).slice_ns;
        if !shorter && self.protected(self.curr) {
            return;
        }
        let eligible = self.eligible(self.ent(woken).vruntime);
        if eligible && (shorter || self.pick(false) == woken) {
            let curr = self.curr;
            let vruntime = self.ent(curr).vruntime;
            self.ent_mut(curr).vprot = vruntime;
            self.resched = true;
        }
    }

    /// Lag de `slot` em ns virtuais: `V - v`. Pronto e corrente; dormindo guarda o seu.
    pub fn lag_ns(&mut self, task: TaskId) -> Option<i64> {
        let slot = slot_u16(self.table.find(task)?);
        let average = self.avg_vruntime();
        let ent = self.ent(slot);
        match ent.state {
            State::Sleeping => Some(ent.vlag),
            _ => Some(signed_diff(average, ent.vruntime)),
        }
    }

    /// Tira uma tarefa pronta ou dormindo da fila para ela entrar em outra (`add_with_lag`),
    /// com o lag que ela tem aqui. A corrente não sai por aqui: `None`.
    pub fn detach(&mut self, task: TaskId) -> Option<Detached> {
        let slot = slot_u16(self.table.find(task)?);
        match self.ent(slot).state {
            State::Running => return None,
            State::Sleeping => self.decay_lag(slot),
            State::Ready => self.leave_competition(slot),
        }
        let ent = *self.ent(slot);
        self.table.release(usize::from(slot));
        Some(Detached {
            params: SchedParams::Fair {
                nice: ent.nice,
                slice_ns: Some(ent.slice_ns),
            },
            lag_ns: ent.vlag,
        })
    }

    /// `add` de quem vem de outra fila: entra com o lag que levava, sem meia fatia de brinde.
    /// # Errors
    /// `WrongClass`, `Full` ou `Duplicate`.
    pub fn add_with_lag(
        &mut self,
        task: TaskId,
        params: SchedParams,
        lag_ns: i64,
        now_ns: u64,
    ) -> Result<(), SchedError> {
        let SchedParams::Fair { nice, slice_ns } = params else {
            return Err(SchedError::WrongClass);
        };
        let slot = slot_u16(self.table.claim(task)?);
        self.update_curr(now_ns);
        let slice = slice_ns
            .unwrap_or(self.config.base_slice_ns)
            .clamp(MIN_SLICE_NS, MAX_SLICE_NS);
        self.max_slice_ns = self.max_slice_ns.max(slice);
        let lag = lag_ns.clamp(
            -self.lag_limit(weight_of_nice(nice)),
            self.lag_limit(weight_of_nice(nice)),
        );
        *self.ent_mut(slot) = Entity {
            slice_ns: slice,
            weight: weight_of_nice(nice),
            nice: nice.clamp(-20, 19),
            vlag: lag,
            ..FRESH
        };
        self.place(slot, false);
        self.enqueue(slot);
        self.check_preempt(slot);
        Ok(())
    }

    /// Uma tarefa pronta (nunca a corrente) que `accept` aceita, a de prazo mais distante:
    /// é a que menos espera perder ao trocar de fila. `None` se nenhuma serve.
    pub fn pull_candidate(&self, mut accept: impl FnMut(TaskId) -> bool) -> Option<TaskId> {
        let mut best: Option<(i64, TaskId)> = None;
        for slot in 0..N {
            let Some(task) = self.table.get(slot) else {
                continue;
            };
            let ent = self.ent(slot_u16(slot));
            if ent.state != State::Ready || !accept(task) {
                continue;
            }
            let distance = signed_diff(ent.deadline, self.zero_vruntime);
            if best.is_none_or(|(farthest, _)| distance > farthest) {
                best = Some((distance, task));
            }
        }
        best.map(|(_, task)| task)
    }

    fn leave_competition(&mut self, slot: u16) {
        match self.ent(slot).state {
            State::Running => {
                self.save_lag(slot);
                self.sum_sub(slot);
                self.curr = NIL;
                self.resched = true;
            }
            State::Ready => {
                self.tree.remove(usize::from(slot));
                self.save_lag(slot);
                self.sum_sub(slot);
            }
            State::Sleeping => {}
        }
    }
}

impl<const N: usize> Default for Eevdf<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Scheduler for Eevdf<N> {
    fn add(&mut self, task: TaskId, params: SchedParams, now_ns: u64) -> Result<(), SchedError> {
        let SchedParams::Fair { nice, slice_ns } = params else {
            return Err(SchedError::WrongClass);
        };
        let slot = self.table.claim(task)?;
        self.update_curr(now_ns);
        let slice = slice_ns
            .unwrap_or(self.config.base_slice_ns)
            .clamp(MIN_SLICE_NS, MAX_SLICE_NS);
        self.max_slice_ns = self.max_slice_ns.max(slice);
        let slot = slot_u16(slot);
        *self.ent_mut(slot) = Entity {
            slice_ns: slice,
            weight: weight_of_nice(nice),
            nice: nice.clamp(-20, 19),
            ..FRESH
        };
        self.place(slot, true);
        self.enqueue(slot);
        self.check_preempt(slot);
        Ok(())
    }

    fn remove(&mut self, task: TaskId) {
        let Some(slot) = self.table.find(task) else {
            return;
        };
        self.leave_competition(slot_u16(slot));
        self.table.release(slot);
    }

    fn wake(&mut self, task: TaskId, now_ns: u64) {
        let Some(slot) = self.table.find(task).map(slot_u16) else {
            return;
        };
        if self.ent(slot).state != State::Sleeping {
            return;
        }
        self.update_curr(now_ns);
        self.decay_lag(slot);
        self.place(slot, false);
        self.enqueue(slot);
        self.check_preempt(slot);
    }

    fn sleep(&mut self, task: TaskId, now_ns: u64) {
        let Some(slot) = self.table.find(task).map(slot_u16) else {
            return;
        };
        self.update_curr(now_ns);
        self.leave_competition(slot);
        self.ent_mut(slot).state = State::Sleeping;
    }

    fn pick_next(&mut self, now_ns: u64) -> Option<TaskId> {
        self.update_curr(now_ns);
        self.resched = false;
        if self.runnable == 0 {
            return None;
        }
        let chosen = if self.runnable == 1 {
            self.pick(false)
        } else {
            self.pick(true)
        };
        let chosen = if chosen == NIL { self.curr } else { chosen };
        if chosen != self.curr {
            self.put_prev(now_ns);
            self.tree.remove(usize::from(chosen));
            self.ent_mut(chosen).state = State::Running;
            self.curr = chosen;
        }
        self.curr_start_ns = now_ns;
        let ent = *self.ent(chosen);
        self.ent_mut(chosen).vprot = ent.deadline;
        self.table.get(usize::from(chosen))
    }

    fn tick(&mut self, now_ns: u64) -> bool {
        self.update_curr(now_ns);
        self.needs_resched()
    }

    /// `yield_task_fair`: o corrente elegível abre mão do que falta da fatia.
    fn yield_current(&mut self, now_ns: u64) {
        self.update_curr(now_ns);
        if self.curr == NIL || self.runnable <= 1 {
            return;
        }
        let slot = self.curr;
        let ent = *self.ent(slot);
        if self.eligible(ent.vruntime) {
            let skipped = signed_diff(ent.deadline, ent.vruntime)
                .max(0)
                .cast_unsigned();
            self.sum_w_vruntime += i128::from(skipped) * i128::from(ent.weight);
            self.ent_mut(slot).vruntime = ent.deadline;
            self.refresh_deadline(slot);
        }
        self.resched = true;
    }

    fn next_deadline_ns(&self, now_ns: u64) -> Option<u64> {
        if self.curr == NIL || self.tree.is_empty() {
            return None;
        }
        let ent = self.ent(self.curr);
        let remaining = signed_diff(ent.deadline, ent.vruntime)
            .max(0)
            .cast_unsigned();
        let at = self
            .curr_start_ns
            .saturating_add(virtual_to_real(remaining, ent.weight));
        Some(at.max(now_ns))
    }

    fn needs_resched(&self) -> bool {
        self.resched || self.curr == NIL && self.runnable > 0
    }

    fn current(&self) -> Option<TaskId> {
        if self.curr == NIL {
            return None;
        }
        self.table.get(usize::from(self.curr))
    }
}

#[cfg(test)]
impl<const N: usize> Eevdf<N> {
    fn live_slots(&self) -> impl Iterator<Item = u16> + '_ {
        (0..N)
            .filter(|slot| self.table.get(*slot).is_some())
            .map(slot_u16)
    }

    /// O mesmo que `pick(false)`, por varredura: o oráculo da árvore.
    fn pick_scan(&self) -> u16 {
        let best = self
            .live_slots()
            .filter(|s| self.ent(*s).state == State::Ready && self.eligible(self.ent(*s).vruntime))
            .min_by_key(|s| {
                let ent = self.ent(*s);
                (
                    ent.deadline.wrapping_sub(self.zero_vruntime).cast_signed(),
                    *s,
                )
            });
        let curr =
            (self.curr != NIL && self.eligible(self.ent(self.curr).vruntime)).then_some(self.curr);
        match (best, curr) {
            (None, curr) => curr.unwrap_or(NIL),
            (Some(best), None) => best,
            (Some(best), Some(curr)) => {
                if self.curr_wins(curr, best) {
                    curr
                } else {
                    best
                }
            }
        }
    }

    fn curr_wins(&self, curr: u16, other: u16) -> bool {
        signed_diff(self.ent(curr).deadline, self.ent(other).deadline) < 0
    }

    /// Recalcula tudo do zero e compara com o que foi mantido de forma incremental.
    pub(crate) fn check_invariants(&self) {
        let (mut weight, mut sum, mut runnable, mut ready) = (0_i64, 0_i128, 0, 0);
        for slot in self.live_slots() {
            let ent = self.ent(slot);
            if ent.state == State::Sleeping {
                continue;
            }
            runnable += 1;
            ready += usize::from(ent.state == State::Ready);
            weight += i64::from(ent.weight);
            sum += self.key(ent.vruntime) * i128::from(ent.weight);
            assert_eq!(ent.state == State::Running, slot == self.curr);
        }
        assert_eq!(runnable, self.runnable, "runnable");
        assert_eq!(weight, self.sum_weight, "sum_weight");
        assert_eq!(sum, self.sum_w_vruntime, "sum_w_vruntime");
        assert_eq!(self.tree.check(), ready, "tree size");
        if weight > 0 {
            let w = i128::from(weight);
            let lag_sum = w * sum.div_euclid(w) - sum;
            assert!((1 - w..=0).contains(&lag_sum), "soma dos lags {lag_sum}");
        }
        let eligible_exists = self
            .live_slots()
            .any(|s| self.ent(s).state != State::Sleeping && self.eligible(self.ent(s).vruntime));
        assert_eq!(eligible_exists, runnable > 0, "sempre há um elegível");
        if runnable > 0 {
            assert_eq!(self.pick(false), self.pick_scan(), "pick contra varredura");
        }
    }

    pub(crate) fn pick_for_bench(&self, scan: bool) -> u16 {
        if scan {
            self.pick_scan()
        } else {
            self.pick(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: u32) -> TaskId {
        TaskId::new(raw)
    }

    fn fair() -> SchedParams {
        SchedParams::fair(0)
    }

    #[test]
    fn rejects_fifo_params() {
        let mut sched = Eevdf::<4>::new();
        assert_eq!(
            sched.add(id(1), SchedParams::fifo(10), 0),
            Err(SchedError::WrongClass)
        );
        assert!(!sched.contains(id(1)));
    }

    #[test]
    fn single_task_runs_forever_without_a_timer() {
        let mut sched = Eevdf::<4>::new();
        sched.add(id(1), fair(), 0).unwrap();
        assert_eq!(sched.pick_next(0), Some(id(1)));
        assert_eq!(sched.next_deadline_ns(0), None);
        assert!(!sched.tick(50_000_000));
        assert_eq!(sched.pick_next(50_000_000), Some(id(1)));
        sched.check_invariants();
    }

    #[test]
    fn all_sleeping_means_idle() {
        let mut sched = Eevdf::<4>::new();
        sched.add(id(1), fair(), 0).unwrap();
        sched.add(id(2), fair(), 0).unwrap();
        assert_eq!(sched.pick_next(0), Some(id(1)));
        sched.sleep(id(1), 10);
        sched.sleep(id(2), 10);
        assert_eq!(sched.pick_next(10), None);
        assert_eq!(sched.current(), None);
        assert!(!sched.needs_resched());
        assert_eq!(sched.next_deadline_ns(10), None);
        sched.check_invariants();
        sched.wake(id(2), 20);
        assert!(sched.needs_resched());
        assert_eq!(sched.pick_next(20), Some(id(2)));
    }

    #[test]
    fn full_capacity_is_an_error_and_a_free_slot_is_reused() {
        let mut sched = Eevdf::<2>::new();
        sched.add(id(1), fair(), 0).unwrap();
        sched.add(id(2), fair(), 0).unwrap();
        assert_eq!(sched.add(id(3), fair(), 0), Err(SchedError::Full));
        assert_eq!(sched.add(id(2), fair(), 0), Err(SchedError::Duplicate));
        sched.remove(id(1));
        sched.add(id(3), fair(), 0).unwrap();
        sched.check_invariants();
    }

    #[test]
    fn unknown_ids_are_ignored() {
        let mut sched = Eevdf::<2>::new();
        sched.add(id(1), fair(), 0).unwrap();
        sched.wake(id(9), 0);
        sched.sleep(id(9), 0);
        sched.remove(id(9));
        assert_eq!(sched.lag_ns(id(9)), None);
        assert_eq!(sched.pick_next(0), Some(id(1)));
        sched.check_invariants();
    }

    #[test]
    fn removing_the_current_task_frees_the_cpu_and_the_weight() {
        let mut sched = Eevdf::<4>::new();
        sched.add(id(1), fair(), 0).unwrap();
        sched.add(id(2), fair(), 0).unwrap();
        assert_eq!(sched.pick_next(0), Some(id(1)));
        sched.remove(id(1));
        assert_eq!(sched.current(), None);
        assert!(sched.needs_resched());
        assert_eq!(sched.runnable(), 1);
        sched.check_invariants();
        assert_eq!(sched.pick_next(10), Some(id(2)));
        sched.remove(id(2));
        assert_eq!(sched.pick_next(20), None);
        sched.check_invariants();
    }

    #[test]
    fn deadline_is_vruntime_plus_slice_over_weight_and_new_tasks_get_half() {
        let mut sched = Eevdf::<4>::new();
        sched.add(id(1), SchedParams::fair(0), 0).unwrap();
        let slot = sched.table.find(id(1)).map(slot_u16).unwrap();
        assert_eq!(sched.ent(slot).deadline, BASE_SLICE_NS / 2);
        sched.add(id(2), SchedParams::fair(-5), 0).unwrap();
        let slot = sched.table.find(id(2)).map(slot_u16).unwrap();
        let expected = real_to_virtual(BASE_SLICE_NS, 3121) / 2;
        assert_eq!(
            sched.ent(slot).deadline,
            sched.ent(slot).vruntime + expected
        );
    }

    #[test]
    fn picks_the_eligible_task_with_the_earliest_deadline() {
        let mut sched = Eevdf::<4>::new();
        sched
            .add(id(1), SchedParams::fair_with_slice(0, 3_000_000), 0)
            .unwrap();
        sched
            .add(id(2), SchedParams::fair_with_slice(0, 1_000_000), 0)
            .unwrap();
        sched
            .add(id(3), SchedParams::fair_with_slice(0, 2_000_000), 0)
            .unwrap();
        assert_eq!(sched.pick_next(0), Some(id(2)));
        sched.check_invariants();
    }

    #[test]
    fn run_to_parity_keeps_the_current_task_until_its_deadline() {
        let mut sched = Eevdf::<4>::new();
        sched
            .add(id(1), SchedParams::fair_with_slice(0, 1_000_000), 0)
            .unwrap();
        sched
            .add(id(2), SchedParams::fair_with_slice(0, 1_000_000), 0)
            .unwrap();
        assert_eq!(sched.pick_next(0), Some(id(1)));
        sched
            .add(id(3), SchedParams::fair_with_slice(0, 1_000_000), 100)
            .unwrap();
        assert!(!sched.needs_resched(), "nova tarefa não fura a fatia");
        assert!(!sched.tick(400_000));
        assert!(sched.tick(1_000_000));
        assert_ne!(sched.pick_next(1_000_000), Some(id(1)));
        sched.check_invariants();
    }

    #[test]
    fn a_shorter_slice_wakeup_preempts_the_protected_current() {
        let mut sched = Eevdf::<4>::new();
        sched.add(id(1), SchedParams::fair(0), 0).unwrap();
        sched
            .add(id(2), SchedParams::fair_with_slice(0, 100_000), 0)
            .unwrap();
        assert_eq!(sched.pick_next(0), Some(id(2)));
        sched.sleep(id(2), 0);
        assert_eq!(sched.pick_next(0), Some(id(1)));
        sched.tick(50_000);
        sched.wake(id(2), 60_000);
        assert!(sched.needs_resched());
        assert_eq!(sched.pick_next(60_000), Some(id(2)));
        sched.check_invariants();
    }

    #[test]
    fn an_equal_slice_wakeup_does_not_preempt_inside_the_slice() {
        let mut sched = Eevdf::<4>::new();
        sched.add(id(1), fair(), 0).unwrap();
        sched.add(id(2), fair(), 0).unwrap();
        assert_eq!(sched.pick_next(0), Some(id(1)));
        sched.sleep(id(1), 10_000);
        assert_eq!(sched.pick_next(10_000), Some(id(2)));
        sched.wake(id(1), 20_000);
        assert!(!sched.needs_resched());
        sched.check_invariants();
    }

    #[test]
    fn next_deadline_is_when_the_current_slice_runs_out() {
        let mut sched = Eevdf::<4>::new();
        sched
            .add(id(1), SchedParams::fair_with_slice(0, 1_000_000), 0)
            .unwrap();
        sched
            .add(id(2), SchedParams::fair_with_slice(0, 1_000_000), 0)
            .unwrap();
        assert_eq!(sched.pick_next(0), Some(id(1)));
        assert_eq!(sched.next_deadline_ns(0), Some(500_000));
        assert!(sched.tick(500_000));
        assert_eq!(sched.pick_next(500_000), Some(id(2)));
        assert_eq!(sched.next_deadline_ns(500_000), Some(1_000_000));
    }

    #[test]
    fn heavier_tasks_get_a_later_virtual_deadline_per_real_ns_run() {
        let mut sched = Eevdf::<4>::new();
        sched.add(id(1), SchedParams::fair(-5), 0).unwrap();
        sched.add(id(2), SchedParams::fair(5), 0).unwrap();
        assert_eq!(sched.pick_next(0), Some(id(1)));
        sched.tick(1_000_000);
        let heavy = sched.lag_ns(id(1)).unwrap();
        let light = sched.lag_ns(id(2)).unwrap();
        assert!(heavy < light, "{heavy} {light}");
        sched.check_invariants();
    }

    fn spin(sched: &mut Eevdf<8>, now: &mut u64, steps: u32) {
        for _ in 0..steps {
            *now += 300_000;
            sched.tick(*now);
            if sched.needs_resched() {
                sched.pick_next(*now);
            }
        }
    }

    fn four_runners() -> (Eevdf<8>, u64) {
        let mut sched = Eevdf::<8>::new();
        for raw in 1..=4 {
            sched.add(id(raw), fair(), 0).unwrap();
        }
        let mut now = 0;
        spin(&mut sched, &mut now, 40);
        (sched, now)
    }

    #[test]
    fn positive_lag_survives_a_sleep_and_wake_cycle() {
        let (mut sched, mut now) = four_runners();
        let sleeper = id(3);
        sched.sleep(sleeper, now);
        let slot = sched.table.find(sleeper).map(slot_u16).unwrap();
        sched.ent_mut(slot).vlag = 600_000;
        spin(&mut sched, &mut now, 40);
        sched.wake(sleeper, now);
        let after = sched.lag_ns(sleeper).unwrap();
        assert!((600_000 - after).abs() <= 16, "depois {after}");
        sched.check_invariants();
    }

    #[test]
    fn negative_lag_burns_off_with_the_virtual_time_slept() {
        let (mut sched, mut now) = four_runners();
        let sleeper = id(3);
        sched.sleep(sleeper, now);
        let slot = sched.table.find(sleeper).map(slot_u16).unwrap();
        sched.ent_mut(slot).vlag = -600_000;
        spin(&mut sched, &mut now, 1);
        sched.wake(sleeper, now);
        let early = sched.lag_ns(sleeper).unwrap();
        assert!(
            (-600_000..-300_000).contains(&early),
            "acordou cedo: {early}"
        );
        sched.sleep(sleeper, now);
        let slot = sched.table.find(sleeper).map(slot_u16).unwrap();
        sched.ent_mut(slot).vlag = -600_000;
        spin(&mut sched, &mut now, 80);
        sched.wake(sleeper, now);
        assert!(
            sched.lag_ns(sleeper).unwrap().abs() <= 16,
            "dormiu muito: lag zerado"
        );
        sched.check_invariants();
    }

    #[test]
    fn lag_is_clamped_to_two_slices() {
        let mut sched = Eevdf::<4>::new();
        sched.add(id(1), fair(), 0).unwrap();
        sched.add(id(2), fair(), 0).unwrap();
        assert_eq!(sched.pick_next(0), Some(id(1)));
        sched.sleep(id(2), 0);
        sched.tick(1_000_000_000);
        sched.pick_next(1_000_000_000);
        sched.sleep(id(1), 1_000_000_000);
        sched.wake(id(2), 1_000_000_000);
        assert!(sched.lag_ns(id(2)).unwrap().abs() <= 2 * BASE_SLICE_NS.cast_signed());
    }

    #[test]
    fn yield_forfeits_the_rest_of_the_slice_only_when_eligible() {
        let mut sched = Eevdf::<4>::new();
        sched.add(id(1), fair(), 0).unwrap();
        sched.add(id(2), fair(), 0).unwrap();
        assert_eq!(sched.pick_next(0), Some(id(1)));
        sched.yield_current(10_000);
        assert!(sched.needs_resched());
        assert_eq!(sched.pick_next(10_000), Some(id(2)));
        sched.check_invariants();
    }

    #[test]
    fn vruntime_wraparound_keeps_the_order() {
        let mut sched = Eevdf::<4>::new();
        sched.zero_vruntime = u64::MAX - 1_000_000;
        sched.add(id(1), fair(), 0).unwrap();
        sched.add(id(2), fair(), 0).unwrap();
        let mut now = 0;
        let mut served = [0_u64; 2];
        for _ in 0..200 {
            let cur = sched.pick_next(now).unwrap();
            now += 100_000;
            served[cur.as_u32() as usize - 1] += 100_000;
            sched.tick(now);
        }
        assert!(sched.zero_vruntime < u64::MAX / 2, "deu a volta");
        assert!(served[0].abs_diff(served[1]) <= 2_000_000, "{served:?}");
        sched.check_invariants();
    }
}
