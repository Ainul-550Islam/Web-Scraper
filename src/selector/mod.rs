//! Hierarchical selectors separate authored extraction shape from transport.
pub mod ast;
pub mod binding;
pub mod engine;
pub mod pagination;
pub mod parser;
pub mod record;
pub use ast::{SelectorKind, SelectorNode, SelectorTree, Transform};
pub use engine::{extract, extract_values};
pub use record::Record;
