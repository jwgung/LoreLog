//! The librarian (사서): an index over every record, searched across all days. It finds and does not judge.
pub mod asks;
pub mod cases;
pub mod eval;
pub mod extract;
pub mod index;
pub mod records;

pub use index::{Exclude, Librarian, Outdated, Report};
