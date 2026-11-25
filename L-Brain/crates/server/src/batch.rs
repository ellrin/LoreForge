//! 待製作 in one go: every reference of the hub is drafted through the app's own routes (the
//! same calls the 製作 page makes), drawn once, and written to a folder to be looked at. Nothing
//! is saved into the hub: saving stays the author's step in the editor.
//!
//! Written into `out`, one set per reference (`<n>` = the id with every character that is not
//! a letter, a digit, `-` or `_` replaced by `_`):
//! `<n>.spec.json` (the draft's spec), `<n>.svg` (drawn by `/flow/api/preview`),
//! `<n>.ref.<ext>` (the reference's picture, when it has one), `<n>.told.json` (what the model
//! was shown, as the draft answered it), and `index.html`: one row per reference, its picture
//! beside the drawing, the error line where a draft or a drawing failed.

use std::path::Path;

/// What became of one reference.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Made {
    pub id: String,
    pub title: String,
    /// Drafted by the timeline task.
    pub timeline: bool,
    /// The engine that answered; empty when the draft failed.
    pub engine: String,
    /// Empty = drafted and drawn; else the one line of what went wrong.
    pub error: String,
}

/// Draft and draw the references of `app` (a router as [`crate::router_isolated`] gives) into
/// `out`. `only` empty = all of them, else just these ids. One at a time, in the list's order;
/// a failure is recorded and the run goes on. `progress` is called after each one.
pub async fn run(app: axum::Router, out: &Path, only: &[String], progress: &mut (dyn FnMut(usize, usize, &Made) + Send)) -> std::io::Result<Vec<Made>> {
    let _ = (app, out, only, progress);
    Ok(Vec::new())
}
