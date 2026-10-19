//! Shared serialized reader and settings types; no database or process dependencies.
mod settings;
pub use settings::*;
mod reader;
pub use reader::*;
mod status;
pub use status::*;
mod checks;
pub use checks::*;
