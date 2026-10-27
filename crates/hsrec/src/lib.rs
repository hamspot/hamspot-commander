//! HamSpot Commander REC: raw channelised IQ recordings, their SQLite
//! index, replay through the current demod chain, derived cache, archive,
//! export, and the timeline layout. No GUI dependencies, so the
//! `commander-rec` CLI can share it.
pub mod archive;
pub mod cache;
pub mod db;
pub mod dsp;
pub mod export;
pub mod lock;
pub mod meta;
pub mod replay;
pub mod safepath;
pub mod segment;
pub mod storage;
pub mod timeline;
pub mod writer;
