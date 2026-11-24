//! Programa de teste do framebuffer como VMO (SDD 18, caso 6, fatia 1a): lê o `boot_kind::FRAMEBUFFER`
//! e o formato, mapeia a tela, pinta um padrão nas primeiras linhas e avisa o kernel. Quando o kernel
//! responde, confere que o padrão segue como o pintado (o console não desenhou por cima) e sai.

#![no_std]
#![no_main]
#![deny(unsafe_code)]

use rustos_user_rt::vm::Mapping;
use rustos_user_rt::{
    Env, FramebufferFormat, Handle, boot, entry, println, signals, sys, vmar_flags,
};

entry!(main);

/// Página do VMO e do mapeamento.
const PAGE: u64 = 4096;
/// Linhas do padrão: cobrem a barra do console e a primeira linha da grade.
const PATTERN_ROWS: usize = 64;
const MAPPED: &[u8] = b"mapped";
const GO: &[u8] = b"go";

/// O pixel `(x, y)` do padrão em XRGB8888 (little-endian): azul = x, verde = y, vermelho cheio.
fn pattern(x: usize, y: usize) -> [u8; 4] {
    [x.to_le_bytes()[0], y.to_le_bytes()[0], 0xFF, 0]
}

/// Largura em pixels, linhas do padrão e `stride` em bytes, como `usize`.
fn span(format: &FramebufferFormat) -> Option<(usize, usize, usize)> {
    let width = usize::try_from(format.width).ok()?;
    let height = usize::try_from(format.height).ok()?;
    let stride = usize::try_from(format.stride).ok()?;
    Some((width, height.min(PATTERN_ROWS), stride))
}

/// Pinta o padrão nas linhas de cima da tela mapeada.
fn paint(bytes: &mut [u8], format: &FramebufferFormat) -> Option<()> {
    let (width, rows, stride) = span(format)?;
    for (y, line) in bytes.chunks_exact_mut(stride).take(rows).enumerate() {
        let pixels = line.get_mut(..width * 4)?.as_chunks_mut::<4>().0;
        for (x, pixel) in pixels.iter_mut().enumerate() {
            *pixel = pattern(x, y);
        }
    }
    Some(())
}

/// Confere que as linhas de cima da tela ainda têm o padrão pintado.
fn kept(bytes: &[u8], format: &FramebufferFormat) -> bool {
    let Some((width, rows, stride)) = span(format) else {
        return false;
    };
    bytes
        .chunks_exact(stride)
        .take(rows)
        .enumerate()
        .all(|(y, line)| {
            line.get(..width * 4).is_some_and(|line| {
                line.as_chunks::<4>()
                    .0
                    .iter()
                    .enumerate()
                    .all(|(x, pixel)| *pixel == pattern(x, y))
            })
        })
}

/// Espera a resposta `go` do kernel no canal de boot.
fn wait_go(channel: Handle) -> bool {
    if sys::object_wait_one(channel, signals::READABLE, u64::MAX).is_err() {
        return false;
    }
    let mut got = [0_u8; 8];
    sys::channel_read(channel, &mut got, &mut [])
        .is_ok_and(|actual| actual.bytes == 2 && got[..2] == *GO)
}

fn main(env: &Env) -> i64 {
    let Some(boot) = boot::attach_console(env) else {
        return 100;
    };
    let Some((screen, format)) = boot.framebuffer else {
        println!("fb: no framebuffer was handed over");
        return 1;
    };
    let (Some(vmar), Some(channel)) = (boot.vmar_root, env.boot_channel()) else {
        return 2;
    };
    let Ok(len) = usize::try_from(format.len_bytes().next_multiple_of(PAGE)) else {
        return 3;
    };
    let Ok(mut map) = Mapping::new(vmar, screen, 0, len, vmar_flags::READ | vmar_flags::WRITE)
    else {
        println!("fb: vmar_map refused the framebuffer");
        return 4;
    };
    if paint(map.bytes_mut(), &format).is_none() {
        return 5;
    }
    if sys::channel_write(channel, MAPPED, &[]).is_err() || !wait_go(channel) {
        return 6;
    }
    let intact = kept(map.bytes(), &format);
    drop(map);
    println!(
        "fb: {} ({}x{}, stride {}, pattern {} rows)",
        if intact { "ok" } else { "FAILED" },
        format.width,
        format.height,
        format.stride,
        PATTERN_ROWS
    );
    if intact { 0 } else { 7 }
}
