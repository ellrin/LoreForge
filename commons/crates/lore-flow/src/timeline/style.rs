//! The style file (package T-style). Signatures are frozen; see `timeline/mod.rs`.

use std::path::Path;

use super::Style;

/// Parse one style file. `stem` = the file name without `.toml`, used as the id when the file
/// gives none. Unknown tables and keys are kept in [`Style::other`]; a value that is not TOML at
/// all, or a known field of the wrong type, is an `Err` of one line. [`check`] is run on the result.
pub fn parse(text: &str, stem: &str) -> Result<Style, String> {
    let _ = (text, stem);
    Err("not built yet".into())
}

/// Fill [`Style::cannot`]: every field whose value is not in [`super::can`], every colour that
/// is neither `point` / `paper` / `none` (where the field allows it) nor six hex digits, an
/// empty palette, a size that is not positive, and every table in [`Style::other`]. One short
/// line each, `table.key = "value"`. Returns the same list.
pub fn check(style: &mut Style) -> Vec<String> {
    style.cannot.clone()
}

/// The neutral styles ([`super::BUILTIN`]), parsed; one that does not parse is a bug and panics
/// in tests, and is skipped at run time.
pub fn builtin() -> Vec<Style> {
    vec![]
}

/// `<dir>/timelines/*.toml`, sorted by file name, each marked [`Style::private`]. A file that
/// does not parse is skipped and its name and error returned in the second list.
pub fn load_dir(dir: &Path) -> (Vec<Style>, Vec<String>) {
    let _ = dir;
    (vec![], vec![])
}

/// The style a spec asks for: by id, else the first of `styles`, else [`Style::default`].
pub fn pick<'a>(styles: &'a [Style], id: &str) -> std::borrow::Cow<'a, Style> {
    match styles.iter().find(|s| s.id == id).or(styles.first()) {
        Some(s) => std::borrow::Cow::Borrowed(s),
        None => std::borrow::Cow::Owned(Style::default()),
    }
}
