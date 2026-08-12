//! The verbs — one module each, all of them taking their roots as arguments.
//!
//! Nothing here reads the environment for a data root: `main` resolves the
//! roots once (`crate::paths`) and hands them down, which is what lets the
//! tests drive every verb against a temp directory.

pub mod dream;
pub mod forget;
pub mod recall;
pub mod reflect;
pub mod remember;
pub mod take;
