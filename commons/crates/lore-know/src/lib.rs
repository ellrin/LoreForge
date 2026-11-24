//! lore-know: the author's private know-how as one database (docs/brain-hub.md).
//!
//! * [`types`]: the frozen contract — [`Item`], [`Category`], [`FormatDef`], [`Source`],
//!   [`Proposal`], the [`Format`], [`Adapter`] and [`Sink`] traits, [`MemSink`] for tests.
//! * [`registry`]: categories from the apps' manifests and the author's own (K1).
//! * [`store`]: the hub folder — records, overlays, trash — behind [`Sink`] (K1).
//! * [`index`]: SQLite over the items: sort, filter, search (K1).
//! * [`formats`]: one module per family of formats — `table` (K2), `text` (K3), `figure` (K4).
//! * [`proposal`]: batches an AI or a tool proposes, applied only when approved (K5).
//! * [`intake`]: outside applications — align or independent, whole-set replace, export (K6).
//! * [`select`]: what applies to a step and its content, written into a prompt (K7).
//! * [`decompose`]: a figure's picture into figure / modules / components (K8).
//! * [`hub`]: everything above opened together for an app (`Hub::open`).
//!
//! Every write goes through [`Sink`]. Nothing here reads the real `private/` in tests.

pub mod decompose;
pub mod formats;
pub mod hub;
pub mod inbox;
pub mod inbox_store;
pub mod index;
pub mod intake;
pub mod process_pattern;
pub mod proposal;
pub mod reader;
pub mod registry;
pub mod select;
pub mod stats;
pub mod store;
pub mod types;
pub(crate) mod util;

pub use types::*;
