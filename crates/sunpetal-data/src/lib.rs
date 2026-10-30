//! The game's data, shared by the runtime and the data builder: the ROM's file system, the LZ10
//! compression, the dialogue scripts and the translation that goes into them (docs/TRANSLATION-RECON.md).
pub mod container;
pub mod delta;
pub mod fingerprint;
pub mod dsrom;
pub mod lz10;
pub mod nftr;
pub mod nitrofs;
pub mod report;
pub mod script;
pub mod tools;
pub mod translate;
