//! Types, schema and (later) crypto shared by the daemon, desktop and phone. No I/O except the
//! opt-in `local-client` feature, so this compiles for iOS without a runtime.

pub mod crypto;
pub mod local_api;
pub mod schema;
