//! Autotestes das etiquetas de TLB (SDD 17). `isolation`: dois espaços com a mesma página em `VA` e
//! conteúdos diferentes, trocados numa CPU enquanto outros nascem e morrem; no modo de teste as
//! etiquetas têm 4 bits, a geração vira dezenas de vezes e ninguém lê a página do outro. `switch_ns`:
//! o custo de ir e voltar entre dois espaços, com uma leitura cada.

use alloc::boxed::Box;

use rustos_hal::abi::USER_MIN;
use rustos_hal::{Cpu, PageFlags};
use rustos_mm::UserRange;

use crate::frames::Frames;
use crate::uspace::UserSpace;
use crate::{asid, platform, task};

/// Endereço da página de teste: a primeira página de usuário.
const VA: u64 = USER_MIN;
/// Espaços que nascem e morrem entre as visitas de `isolation`.
const SPACES: u64 = 48;
/// Trocas de ida e volta que `switch_ns` mede.
const SWITCHES: u64 = 2_000;

/// Um espaço com uma página em `VA` que começa com `mark`.
fn make(frames: Frames, map_offset: u64, mark: u64) -> Option<Box<UserSpace>> {
    // SAFETY: o mapa direto cobre todo frame do alocador e esta tarefa roda nas tabelas do kernel.
    let mut space = Box::new(unsafe { UserSpace::new(frames, map_offset) }.ok()?);
    space
        .map_at(VA, 0, &mark.to_le_bytes(), 1, PageFlags::USER_DATA)
        .ok()?;
    Some(space)
}

/// Liga `space` nesta CPU, lê a página e solta o espaço. Devolve o que foi lido (`u64::MAX` se a
/// leitura falhou).
fn visit(space: &mut UserSpace) -> u64 {
    let cpu = platform::cpu();
    task::bind_user(space.root().as_u64(), 0, space.active_addr());
    cpu.restore_interrupts(true);
    let mut bytes = [0_u8; 8];
    let read = match UserRange::new(VA, 8) {
        Ok(range) if platform::copy_from_user(range, &mut bytes) => u64::from_le_bytes(bytes),
        _ => u64::MAX,
    };
    cpu.save_and_disable_interrupts();
    let _ = task::unbind_user();
    read
}

/// Nenhum espaço lê a página de outro, depois de quantas viradas de geração forem: a página de cada um
/// tem a sua marca, e o espaço novo que nasce depois de uma virada não herda a tradução velha.
pub fn isolation(frames: Frames, map_offset: u64) -> bool {
    let (Some(mut first), Some(mut second)) = (
        make(frames, map_offset, 0xA11C_E000),
        make(frames, map_offset, 0xB0B0_0000),
    ) else {
        return false;
    };
    let start = asid::stats().0;
    let mut ok = true;
    for round in 0..SPACES {
        let mark = 0xC000_0000 + round;
        let Some(mut newcomer) = make(frames, map_offset, mark) else {
            return false;
        };
        ok &= visit(&mut newcomer) == mark;
        newcomer.wait_inactive();
        newcomer.destroy();
        ok &= visit(&mut first) == 0xA11C_E000 && visit(&mut second) == 0xB0B0_0000;
    }
    let (generation, full, space) = asid::stats();
    let turned = generation - start;
    let tagged = asid::enabled();
    ok &= !tagged || turned >= 2;
    first.wait_inactive();
    first.destroy();
    second.wait_inactive();
    second.destroy();
    kprintln!(
        "[rustos] asid self-test: {} ({} generations turned, {full} full flushes, {space} tag flushes, {} bits)",
        if ok { "ok" } else { "FAILED" },
        turned,
        asid::bits()
    );
    ok
}

/// Ns por troca de espaço: ida e volta entre dois espaços, cada visita lê a página. Com etiqueta, a
/// troca não esvazia o TLB; sem, cada troca de raiz esvazia.
pub fn switch_ns(frames: Frames, map_offset: u64) -> Option<u64> {
    let mut first = make(frames, map_offset, 1)?;
    let mut second = make(frames, map_offset, 2)?;
    for _ in 0..16 {
        visit(&mut first);
        visit(&mut second);
    }
    let start = platform::cycles();
    for _ in 0..SWITCHES {
        visit(&mut first);
        visit(&mut second);
    }
    let ticks = platform::cycles().saturating_sub(start);
    first.wait_inactive();
    first.destroy();
    second.wait_inactive();
    second.destroy();
    Some(platform::cycles_to_ns(ticks) / (2 * SWITCHES))
}
