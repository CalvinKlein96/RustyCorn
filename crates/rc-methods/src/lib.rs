//! ÄKTAprime method text (method dumps, method files, the PC method store), the run
//! setup kept next to each PC method, which PC method a run started from, and the
//! example methods of a new methods folder.

pub mod examples;
pub mod matching;
pub mod method;
pub mod store;

pub use examples::*;
pub use matching::*;
pub use method::*;
pub use store::*;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_examples;
#[cfg(test)]
mod tests_setup;
