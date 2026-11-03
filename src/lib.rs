//! The recalc core: engine, store, MCP server and CLI. The desktop crate wraps it.

pub mod agent;
pub mod bundle;
pub mod cli;
pub mod crash;
pub mod danger;
pub mod demos;
pub mod engine;
pub mod exec;
pub mod expr;
pub mod hook;
pub mod interp;
pub mod mcp;
pub mod model;
pub mod runner;
pub mod session;
pub mod shell_env;
pub mod source;
pub mod store;
pub mod stream;
