//! life server: the raw store, the diary assistant, the librarian and the collectors, as a library. The server binaries
//! (`life-ingest`, `life`, ...) and the desktop app run the same code.
pub mod assistant;
pub mod config;
pub mod filestore;
pub mod fsutil;
pub mod http;
pub mod jobs;
pub mod llm;
pub mod store;
