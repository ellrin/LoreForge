//! lore-canvas — app-agnostic slide canvas core (design §2.2, §7.3, §7.8).
//!
//! * [`model`]: scene model (deck → slides → objects; text with tag/emphasis
//!   runs; shapes, connectors, pictures, groups, tables, equations, locked
//!   objects), EMU frames, fingerprints for lossless export.
//! * [`style`]: style-pack rules (tags, palette, emphasis, ghost rules).
//! * [`ops`]: undoable JSON-patch-like operations ([`ops::Editor`]).
//! * [`geom`]: bounds, connection sites, connector routing, align/distribute.
//! * [`reveal`]: progressive-reveal generator.
//! * [`components`]: style-pack components → objects.
//! * [`pptx_in`] / [`pptx_out`]: lore-ooxml ↔ canvas (untouched objects keep
//!   their original XML; unedited slides stay byte-identical).

pub mod components;
pub mod geom;
pub mod model;
pub mod ops;
pub mod pptx_in;
pub mod pptx_out;
pub mod reveal;
pub mod style;

pub use model::*;
pub use ops::{Applied, Editor, Op, OpError};
pub use style::StyleSystem;
