//! The librarian (사서): an index over every record, searched across all days. It finds and does not judge.
pub mod index;
pub mod records;

pub use index::{Exclude, Librarian, Outdated, Report};
