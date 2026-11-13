//! The F3 viewer without the UI.
//!
//! - [`doc::Doc`]: a file read in blocks (never whole) through a block cache
//!   with a memory limit, read-ahead, change watching and continuing after a
//!   lost connection; the line index is built in the background.
//! - [`rows`]: lines split into rows; extremely long lines are cut into
//!   segments, so every row is short and found from any position by looking
//!   at most a few kilobytes back.
//! - [`enc`]: encodings (detection, decoding with source positions).

pub mod cache;
pub mod doc;
pub mod enc;
pub mod hex;
pub mod index;
pub mod rows;
pub mod search;
pub mod syntax;

pub use enc::{Detected, Enc};
