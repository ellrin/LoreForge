//! The web files, compiled in (like lore-shell): `commons/ui/plot/*` and the KaTeX copy in
//! `commons/ui/notation-panel/vendor/katex`, served under `/plot/`.

/// `(path under /plot/, content type, bytes)`.
pub const FILES: &[(&str, &str, &[u8])] = &[
    ("index.html", "text/html; charset=utf-8", include_bytes!("../web/index.html")),
    ("mathsvg.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/mathsvg.js")),
    ("plot.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/plot/plot.js")),
    ("charts.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/plot/charts.js")),
    ("charts3d.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/plot/charts3d.js")),
    ("eqcore.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/plot/eqcore.js")),
    ("simcore.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/plot/simcore.js")),
    ("equation.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/plot/equation.js")),
    ("eqlive.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/plot/eqlive.js")),
    ("eqsim.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/plot/eqsim.js")),
    ("equations.json", "application/json", include_bytes!("../../../ui/plot/equations.json")),
    ("formats.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/plot/formats.js")),
    ("formats.json", "application/json", include_bytes!("../../../ui/plot/formats.json")),
    ("palettes.json", "application/json", include_bytes!("../../../ui/plot/palettes.json")),
    ("editor.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/plot/editor.js")),
    ("editor.css", "text/css; charset=utf-8", include_bytes!("../../../ui/plot/editor.css")),
    ("samples.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/plot/samples.js")),
    ("gallery.html", "text/html; charset=utf-8", include_bytes!("../../../ui/plot/gallery.html")),
    ("katex/katex.min.js", "text/javascript; charset=utf-8", include_bytes!("../../../ui/notation-panel/vendor/katex/katex.min.js")),
    ("katex/katex.min.css", "text/css; charset=utf-8", include_bytes!("../../../ui/notation-panel/vendor/katex/katex.min.css")),
];

/// KaTeX's woff2 fonts (served at `/plot/katex/fonts/<file>`, embedded in exported SVGs).
pub const KATEX_FONTS: &[(&str, &[u8])] = &[
    ("KaTeX_AMS-Regular.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_AMS-Regular.woff2")),
    ("KaTeX_Caligraphic-Bold.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Caligraphic-Bold.woff2")),
    ("KaTeX_Caligraphic-Regular.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Caligraphic-Regular.woff2")),
    ("KaTeX_Fraktur-Bold.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Fraktur-Bold.woff2")),
    ("KaTeX_Fraktur-Regular.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Fraktur-Regular.woff2")),
    ("KaTeX_Main-Bold.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Main-Bold.woff2")),
    ("KaTeX_Main-BoldItalic.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Main-BoldItalic.woff2")),
    ("KaTeX_Main-Italic.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Main-Italic.woff2")),
    ("KaTeX_Main-Regular.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Main-Regular.woff2")),
    ("KaTeX_Math-BoldItalic.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Math-BoldItalic.woff2")),
    ("KaTeX_Math-Italic.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Math-Italic.woff2")),
    ("KaTeX_SansSerif-Bold.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_SansSerif-Bold.woff2")),
    ("KaTeX_SansSerif-Italic.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_SansSerif-Italic.woff2")),
    ("KaTeX_SansSerif-Regular.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_SansSerif-Regular.woff2")),
    ("KaTeX_Script-Regular.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Script-Regular.woff2")),
    ("KaTeX_Size1-Regular.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Size1-Regular.woff2")),
    ("KaTeX_Size2-Regular.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Size2-Regular.woff2")),
    ("KaTeX_Size3-Regular.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Size3-Regular.woff2")),
    ("KaTeX_Size4-Regular.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Size4-Regular.woff2")),
    ("KaTeX_Typewriter-Regular.woff2", include_bytes!("../../../ui/notation-panel/vendor/katex/fonts/KaTeX_Typewriter-Regular.woff2")),
];

/// The scripts a page loads, in order, before `LorePlot.edit` / `LorePlot.render` work.
pub const SCRIPTS: &[&str] = &["mathsvg.js", "plot.js", "charts.js", "charts3d.js", "eqcore.js", "simcore.js", "equation.js", "eqlive.js", "eqsim.js", "formats.js", "editor.js"];

/// A file by its path under `/plot/`.
pub fn get(path: &str) -> Option<(&'static str, &'static [u8])> {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(f) = path.strip_prefix("katex/fonts/") {
        return KATEX_FONTS.iter().find(|(n, _)| *n == f).map(|(_, b)| ("font/woff2", *b));
    }
    FILES.iter().find(|(p, _, _)| *p == path).map(|(_, m, b)| (*m, *b))
}
