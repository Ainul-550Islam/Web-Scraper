# Rust unit tests live in src/<module>/tests.rs file-modules

Status: **Accepted**

## Context & decision

Inline `#[cfg(test)] mod tests { … }` blocks pushed several engine files past 1500 lines. Extracting each block to `src/<stem>/tests.rs` with `#[cfg(test)] mod tests;` in the parent keeps tests adjacent while halving file size. Crate-root tests sit at `src/tests.rs`; an `x.rs` + `x/mod.rs` pair is an E0761 conflict by design, so the `x.rs` + `x/tests.rs` shape is the legal one.
