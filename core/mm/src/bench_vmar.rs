//! Medição no host, só com `--ignored`: `cargo test --release -p rustos-mm bench -- --ignored
//! --nocapture --test-threads=1`. Custo das operações do VMAR com N regiões vivas.

use core::hint::black_box;
use std::time::Instant;

use crate::testing::XorShift;
use crate::vmar::{Access, Attrs, Fit, Kind, Perms, Placement, Region, Vmar};

const PAGE: u64 = 4096;
const BASE: u64 = 0x1_0000;
const TOP: u64 = 0x7FFF_FFFF_F000;
const SIZES: [usize; 3] = [16, 256, 4096];
const ROUNDS: usize = 7;

fn rw() -> Attrs {
    Attrs::new(Kind::Mapping, Perms::READ.union(Perms::WRITE))
}

/// N regiões de 1 página nas páginas pares, buracos de 1 página entre elas e o resto livre
/// depois da última; um slot sobrando para a operação medida.
fn filled(n: usize) -> Vmar<Vec<Region>> {
    let mut vmar = Vmar::new(vec![Region::EMPTY; n + 1], BASE, TOP).unwrap();
    for i in 0..n {
        vmar.reserve(BASE + 2 * i as u64 * PAGE, PAGE, rw())
            .unwrap();
    }
    vmar
}

fn best_ns(ops: usize, mut run: impl FnMut()) -> f64 {
    let mut best = f64::MAX;
    for _ in 0..ROUNDS {
        let start = Instant::now();
        run();
        best =
            best.min(start.elapsed().as_secs_f64() * 1e9 / f64::from(u32::try_from(ops).unwrap()));
    }
    best
}

#[test]
#[ignore = "medição: rode com --release --ignored"]
fn bench_vmar() {
    const OPS: usize = 200_000;
    println!("\n-- vmar: N regiões vivas, melhor de {ROUNDS} rodadas, ns por operação --");
    println!(
        "{:>6} {:>8} {:>16} {:>14} {:>22} {:>22}",
        "N",
        "find",
        "reserve+unmap",
        "falta (fault)",
        "allocate+unmap Lowest",
        "allocate+unmap Random"
    );
    for n in SIZES {
        let mut vmar = filled(n);
        let mut rng = XorShift::new(0xB3_4C);
        let holes: Vec<u64> = (0..OPS)
            .map(|_| BASE + (2 * rng.below(n) as u64 + 1) * PAGE)
            .collect();
        let addrs: Vec<u64> = (0..OPS)
            .map(|_| BASE + rng.below(2 * n) as u64 * PAGE + 8)
            .collect();

        let find = best_ns(OPS, || {
            for &addr in &addrs {
                black_box(vmar.find(black_box(addr)));
            }
        });
        let fault = best_ns(OPS, || {
            for &addr in &addrs {
                let _ = black_box(vmar.check_fault(black_box(addr), Access::Write));
            }
        });
        let pair = best_ns(OPS, || {
            for &hole in &holes {
                vmar.reserve(black_box(hole), PAGE, rw()).unwrap();
                vmar.unmap(black_box(hole), PAGE).unwrap();
            }
        });
        let alloc = |placement, vmar: &mut Vmar<Vec<Region>>, rng: &mut XorShift| {
            let fit = Fit {
                len: 2 * PAGE,
                align: PAGE,
                window: (0, u64::MAX),
                placement,
            };
            let mut draw = || rng.next();
            let start = vmar.allocate(fit, rw(), &mut draw).unwrap();
            vmar.unmap(black_box(start), 2 * PAGE).unwrap();
        };
        let reps = OPS / n.max(16);
        let lowest = best_ns(reps, || {
            for _ in 0..reps {
                alloc(Placement::Lowest, &mut vmar, &mut rng);
            }
        });
        let random = best_ns(reps, || {
            for _ in 0..reps {
                alloc(Placement::Random, &mut vmar, &mut rng);
            }
        });
        println!("{n:>6} {find:>8.1} {pair:>16.1} {fault:>14.1} {lowest:>22.1} {random:>22.1}");
    }
    println!(
        "memória: 32 B por região; N = 16, 256, 4096 -> {} B, {} B, {} B",
        16 * size_of::<Region>(),
        256 * size_of::<Region>(),
        4096 * size_of::<Region>()
    );
}
