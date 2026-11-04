//! Startup order and graceful shutdown for The Entity In The Shed.
//!
//! Stage 1 fills this in: configure logging, validate `AppConfig`, create the storage
//! root, start the background tasks, serve, and answer `--smoke`. Until then the binary
//! exists so the workspace builds and the layering gate can see a package at the top of
//! the dependency graph.

fn main() {}
