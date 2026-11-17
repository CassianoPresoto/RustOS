//! Desenho em framebuffer linear: política pura de pixels, sem `unsafe`.
//! Quem transforma o endereço do framebuffer em fatia é o kernel.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod color;
pub mod font;
pub mod surface;
pub mod text;

pub use color::{Pixel, PixelFormat, Rgb};
pub use surface::{MAX_SCALE, Rect, Surface};
pub use text::Style;
