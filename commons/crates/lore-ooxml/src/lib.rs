//! lore-ooxml — lossless PPTX read/write for SlideComposer.
//!
//! * [`package`]: the zip, every part kept as raw bytes.
//! * [`xml`]: small owned DOM + canonical comparison.
//! * [`model`]: typed slide model (shapes, text, pictures, connectors, groups,
//!   tables, OMML alternate content, placeholders); unknown pieces kept raw.
//! * [`deck`]: slides/layouts/masters/themes, placeholder inheritance, save,
//!   style templates, adding slides.
//! * [`anim`]: the five entrance animations.
//! * [`inventory`]: per-slide object/locked report.
//! * [`style`]: style packs (`styles/<id>/style.toml` + `template.pptx`) and
//!   the statistical extractor.

pub mod anim;
pub mod deck;
pub mod inventory;
pub mod model;
pub mod package;
pub mod shapes;
pub mod style;
pub mod xml;

pub use deck::Deck;
pub use model::{Shape, SlideDoc};
pub use package::Package;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("zip: {0}")]
    Zip(String),
    #[error("xml: {0}")]
    Xml(String),
    #[error("missing part: {0}")]
    MissingPart(String),
    #[error("refusing to overwrite existing file: {0}")]
    Exists(String),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl SlideDoc {
    /// Replace the slide's timing with these entrance animations (empty = remove).
    pub fn set_entrance_animations(&mut self, anims: &[anim::EntranceAnim]) {
        self.timing = if anims.is_empty() {
            None
        } else {
            Some(anim::build_timing(anims))
        };
    }
    pub fn entrance_animations(&self) -> Vec<anim::EntranceAnim> {
        self.timing
            .as_ref()
            .and_then(anim::read_timing)
            .unwrap_or_default()
    }
}
