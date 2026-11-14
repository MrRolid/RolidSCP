//! RolidSCP core: shared models and the virtual filesystem abstraction,
//! independent of UI and concrete backends.

pub mod color;
pub mod copy;
pub mod eta;
pub mod filter;
pub mod names;
pub mod perms;
pub mod rate;
pub mod rename;
pub mod replace;
pub mod vfs;

pub use vfs::{Entry, EntryKind, Vfs, VfsError, VfsResult};
