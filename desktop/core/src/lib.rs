//! life-agent core: turns "what is in front, and how long since the last input" samples into time segments,
//! keeps them until the server has accepted them, and sends them. No OS calls live here, so all of it is tested anywhere.
pub mod agent;
pub mod config;
pub mod segments;
pub mod sender;
pub mod spool;
