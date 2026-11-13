//! Reader of the PS2 disc's data (docs/ULTIMATE-IMPORT.md, track У1): the ISO 9660 file system and,
//! step by step, the formats inside. Read-only; the disc is never written to.
pub mod bpe;
pub mod events;
pub mod gig;
pub mod gim;
pub mod gs;
pub mod iso;
pub mod lf;
pub mod pose;
pub mod scenedb;
pub mod sha1;
pub mod tblock;
