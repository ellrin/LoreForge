//! What the author knows and has read, read-only (docs/paper-deck.md 「不熟」標示, docs/book.md):
//!
//! - [`terms`]: a paper's domain terms against the L-Brain (熟／學習中／不熟) and the
//!   cards' prerequisite links. L-Slide's 論文報告 and L-Paper's 寫書 share it.
//! - [`reader`]: L-Reader's papers, PDFs, 懂了 paragraphs and 問這篇 questions.

pub mod reader;
pub mod terms;

pub use terms::*;
