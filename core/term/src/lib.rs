//! Emulador de terminal: transforma bytes VT/xterm em uma grade de células.
//! Não conhece pixels; quem desenha a grade é o kernel, com o `gfx`.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

mod cell;
mod dirty;
mod key;
mod keyboard;
mod palette;
mod parser;
mod terminal;

pub use cell::{Cell, Flags};
pub use dirty::{DirtyRows, MAX_ROWS};
pub use key::Key;
pub use keyboard::{Keyboard, Layout};
pub use palette::xterm_rgb;
pub use terminal::{Cursor, Terminal};
