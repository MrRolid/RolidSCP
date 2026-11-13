//! Disks of this computer and of servers: which file system a path is on
//! and the health (SMART) of the physical disk behind it.

pub mod cache;
pub mod health;
pub mod mounts;
pub mod physical;
pub mod remote;
pub mod smart;
pub mod smartctl;
pub mod udisks;
