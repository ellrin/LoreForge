//! `sc-read-equation`: a display equation's LaTeX against its crop. The reading itself comes
//! from the model that looks at the page; this task compares the typeset reading with the
//! crop symbol by symbol and corrects it (`extract.rs`, `compare`). A reading that never
//! agrees with the crop is shown as the crop, with a warning; a slide never shows LaTeX
//! source as text, and L-Reader's text-layer LaTeX (which can hold control characters) is
//! only ever a hint.

pub const TASK: &str = "sc-read-equation";
