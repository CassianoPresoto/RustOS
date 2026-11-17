//! Treap de prazos virtuais com o menor `vruntime` de cada subárvore: a busca do
//! "elegível de menor prazo" desce uma só vez, O(log n), como a rb-tree do Linux.

use crate::table::{NIL, slot_u16};

#[derive(Clone, Copy, Debug)]
struct Node {
    left: u16,
    right: u16,
    priority: u32,
    deadline: u64,
    vruntime: u64,
    min_vruntime: u64,
}

#[cfg(test)]
pub(crate) const NODE_SIZE: usize = size_of::<Node>();

const EMPTY: Node = Node {
    left: NIL,
    right: NIL,
    priority: 0,
    deadline: 0,
    vruntime: 0,
    min_vruntime: 0,
};

/// Diferença com sinal entre dois tempos virtuais que dão a volta em `u64`.
pub(crate) const fn signed_diff(a: u64, b: u64) -> i64 {
    a.wrapping_sub(b).cast_signed()
}

fn min_virtual(a: u64, b: u64) -> u64 {
    if signed_diff(a, b) <= 0 { a } else { b }
}

fn key_before(deadline_a: u64, slot_a: u16, deadline_b: u64, slot_b: u16) -> bool {
    let diff = signed_diff(deadline_a, deadline_b);
    diff < 0 || (diff == 0 && slot_a < slot_b)
}

#[derive(Debug)]
pub(crate) struct DeadlineTree<const N: usize> {
    nodes: [Node; N],
    root: u16,
    rng: u32,
}

impl<const N: usize> DeadlineTree<N> {
    pub(crate) const fn new() -> Self {
        Self {
            nodes: [EMPTY; N],
            root: NIL,
            rng: 0x9E37_79B9,
        }
    }

    pub(crate) const fn is_empty(&self) -> bool {
        self.root == NIL
    }

    fn node(&self, slot: u16) -> &Node {
        &self.nodes[usize::from(slot)]
    }

    fn node_mut(&mut self, slot: u16) -> &mut Node {
        &mut self.nodes[usize::from(slot)]
    }

    fn next_priority(&mut self) -> u32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        x
    }

    fn pull(&mut self, slot: u16) {
        let node = *self.node(slot);
        let mut min = node.vruntime;
        for child in [node.left, node.right] {
            if child != NIL {
                min = min_virtual(min, self.node(child).min_vruntime);
            }
        }
        self.node_mut(slot).min_vruntime = min;
    }

    fn split(&mut self, tree: u16, deadline: u64, slot: u16) -> (u16, u16) {
        if tree == NIL {
            return (NIL, NIL);
        }
        let node = *self.node(tree);
        if key_before(node.deadline, tree, deadline, slot) {
            let (low, high) = self.split(node.right, deadline, slot);
            self.node_mut(tree).right = low;
            self.pull(tree);
            (tree, high)
        } else {
            let (low, high) = self.split(node.left, deadline, slot);
            self.node_mut(tree).left = high;
            self.pull(tree);
            (low, tree)
        }
    }

    fn merge(&mut self, a: u16, b: u16) -> u16 {
        if a == NIL {
            return b;
        }
        if b == NIL {
            return a;
        }
        if self.node(a).priority > self.node(b).priority {
            let right = self.merge(self.node(a).right, b);
            self.node_mut(a).right = right;
            self.pull(a);
            a
        } else {
            let left = self.merge(a, self.node(b).left);
            self.node_mut(b).left = left;
            self.pull(b);
            b
        }
    }

    fn erase(&mut self, tree: u16, slot: u16, deadline: u64) -> u16 {
        if tree == NIL {
            return NIL;
        }
        let node = *self.node(tree);
        if tree == slot {
            return self.merge(node.left, node.right);
        }
        if key_before(deadline, slot, node.deadline, tree) {
            let left = self.erase(node.left, slot, deadline);
            self.node_mut(tree).left = left;
        } else {
            let right = self.erase(node.right, slot, deadline);
            self.node_mut(tree).right = right;
        }
        self.pull(tree);
        tree
    }

    /// Insere `slot`, que não pode estar na árvore. Ordem: prazo, depois slot.
    pub(crate) fn insert(&mut self, slot: usize, deadline: u64, vruntime: u64) {
        let slot = slot_u16(slot);
        let priority = self.next_priority();
        *self.node_mut(slot) = Node {
            left: NIL,
            right: NIL,
            priority,
            deadline,
            vruntime,
            min_vruntime: vruntime,
        };
        let (low, high) = self.split(self.root, deadline, slot);
        let left = self.merge(low, slot);
        self.root = self.merge(left, high);
    }

    /// Tira `slot`, que tem de estar na árvore.
    pub(crate) fn remove(&mut self, slot: usize) {
        let slot = slot_u16(slot);
        let deadline = self.node(slot).deadline;
        self.root = self.erase(self.root, slot, deadline);
    }

    /// O slot de menor prazo entre os que `eligible(vruntime)` aceita. Vale porque
    /// elegível é monótono no `vruntime`: o mínimo da subárvore decide por ela.
    pub(crate) fn first_eligible(&self, eligible: impl Fn(u64) -> bool) -> Option<usize> {
        let mut tree = self.root;
        while tree != NIL {
            let node = self.node(tree);
            if node.left != NIL && eligible(self.node(node.left).min_vruntime) {
                tree = node.left;
            } else if eligible(node.vruntime) {
                return Some(usize::from(tree));
            } else {
                tree = node.right;
            }
        }
        None
    }

    /// Confere ordem, heap de prioridade e o mínimo por subárvore; devolve a contagem.
    #[cfg(test)]
    pub(crate) fn check(&self) -> usize {
        let mut order = Vec::new();
        self.collect(self.root, &mut order);
        for pair in order.windows(2) {
            let (a, b) = (self.node(pair[0]), self.node(pair[1]));
            assert!(key_before(a.deadline, pair[0], b.deadline, pair[1]));
        }
        order.len()
    }

    #[cfg(test)]
    fn collect(&self, slot: u16, out: &mut Vec<u16>) {
        if slot == NIL {
            return;
        }
        let node = *self.node(slot);
        self.collect(node.left, out);
        out.push(slot);
        self.collect(node.right, out);
        let mut min = node.vruntime;
        for child in [node.left, node.right] {
            if child != NIL {
                assert!(self.node(child).priority <= node.priority, "heap order");
                min = min_virtual(min, self.node(child).min_vruntime);
            }
        }
        assert_eq!(min, node.min_vruntime, "min_vruntime of slot {slot}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brute(entries: &[(usize, u64, u64)], limit: u64) -> Option<usize> {
        entries
            .iter()
            .filter(|(_, _, v)| signed_diff(*v, limit) <= 0)
            .min_by_key(|(slot, deadline, _)| (*deadline, *slot))
            .map(|(slot, _, _)| *slot)
    }

    #[test]
    fn matches_a_linear_scan_under_random_inserts_and_removes() {
        let mut tree = DeadlineTree::<64>::new();
        let mut live: Vec<(usize, u64, u64)> = Vec::new();
        let mut seed = 0x1234_5678_u64;
        let mut rand = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..20_000 {
            let slot = (rand() % 64) as usize;
            if let Some(at) = live.iter().position(|(s, _, _)| *s == slot) {
                tree.remove(slot);
                live.swap_remove(at);
            } else {
                let (deadline, vruntime) = (rand() % 1_000, rand() % 1_000);
                tree.insert(slot, deadline, vruntime);
                live.push((slot, deadline, vruntime));
            }
            assert_eq!(tree.check(), live.len());
            let limit = rand() % 1_000;
            let found = tree.first_eligible(|v| signed_diff(v, limit) <= 0);
            assert_eq!(found, brute(&live, limit), "limit {limit}");
        }
    }

    #[test]
    fn orders_across_the_u64_wraparound() {
        let mut tree = DeadlineTree::<4>::new();
        tree.insert(0, u64::MAX - 5, 0);
        tree.insert(1, 3, 0);
        tree.insert(2, u64::MAX - 100, 0);
        assert_eq!(tree.first_eligible(|_| true), Some(2));
        tree.remove(2);
        assert_eq!(tree.first_eligible(|_| true), Some(0));
        tree.remove(0);
        tree.remove(1);
        assert!(tree.is_empty());
        assert_eq!(tree.first_eligible(|_| true), None);
    }
}
