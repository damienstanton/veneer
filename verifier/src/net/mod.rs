//! veneer net: a harness-agnostic task queue and agent memory shared by every
//! Claude Code and Antigravity session, on any machine or in the cloud.
//!
//! The store is an op-log CRDT (see `spec/net.md`): each replica appends to its
//! own file on a git hub branch, and every replica folds the same ops into the
//! same state. A task is done only with a veneer ship-gate witness.
pub mod envelope;
pub mod cli;
pub mod cli_net;
pub mod config;
pub mod error;
pub mod harness;
pub mod hlc;
pub mod hub;
pub mod mcp;
pub mod memory;
pub mod op;
pub mod replica;
pub mod sync;
pub mod tasks;
pub mod witness;
pub mod worker;
