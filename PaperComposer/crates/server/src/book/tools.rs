//! The outside programs the job needs, behind a trait so tests run without them: lore-plot's
//! headless render and export (Chrome; the same `save_paper_figure` as the editor's 繪圖…) and the
//! book's compile (the same `export::build_pdf` as 匯出 PDF).

use std::path::{Path, PathBuf};

/// What a compile gave.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(default)]
pub struct Compiled {
    /// No TeX here: not compiled (a note, not a problem of the book).
    pub skipped: Option<String>,
    pub ok: bool,
    pub pdf: Option<String>,
    pub pages: Option<u32>,
    pub errors: Vec<String>,
    pub undefined_refs: Vec<String>,
    pub undefined_cites: Vec<String>,
}

#[async_trait::async_trait]
pub trait Tools: Send + Sync {
    /// A lore-plot spec → `figures/<stem>.pdf` + `.plot.json` in `figs`; the file name written.
    fn plot(&self, figs: &Path, stem: &str, spec: &lore_plot::Spec, styles: &serde_json::Value) -> Result<String, String>;
    async fn compile(&self, book: &Path) -> Compiled;
}

pub struct RealTools;

#[async_trait::async_trait]
impl Tools for RealTools {
    fn plot(&self, figs: &Path, stem: &str, spec: &lore_plot::Spec, styles: &serde_json::Value) -> Result<String, String> {
        let tools = lore_plot::Tools::find();
        let svg = lore_plot::export::render_spec(&tools, &spec.to_json(), styles).map_err(|e| e.to_string())?;
        let saved = lore_plot::figure::save_paper_figure(&tools, figs, stem, false, spec, &svg).map_err(|e| e.to_string())?;
        Ok(saved.file)
    }

    async fn compile(&self, book: &Path) -> Compiled {
        let main = std::fs::read_to_string(pc_core::open::main_file(book)).unwrap_or_default();
        let engine = crate::export::engine_for(&main);
        if crate::texmeasure::tool(engine).is_none() {
            return Compiled { skipped: Some(format!("這台電腦沒有 {engine}，沒有編譯")), ..Default::default() };
        }
        let name = book.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "book".into());
        let name: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
        match crate::export::build_pdf(book, &name, false).await {
            Ok(r) => Compiled {
                skipped: None,
                ok: r.ok,
                pdf: r.file.map(|f| book.join(f).to_string_lossy().to_string()),
                pages: r.pages,
                errors: r.errors.iter().map(|e| format!("{}{}", e.file.as_deref().map(|f| format!("{f}: ")).unwrap_or_default(), e.message)).collect(),
                undefined_refs: r.undefined_refs,
                undefined_cites: r.undefined_cites,
            },
            Err(e) => Compiled { errors: vec![e], ..Default::default() },
        }
    }
}

/// A one-page blank PDF that TeX can include (tests): objects, a correct xref table, trailer.
pub fn tiny_pdf() -> Vec<u8> {
    let objs = ["<</Type/Catalog/Pages 2 0 R>>", "<</Type/Pages/Kids[3 0 R]/Count 1>>", "<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 120]/Resources<<>>/Contents 4 0 R>>", "<</Length 23>>\nstream\n0.5 g 20 20 160 80 re f\nendstream"];
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in offsets {
        out.extend(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend(format!("trailer\n<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// Tests: figures are a tiny PDF next to the spec; the compile runs XeLaTeX only when `compile` is
/// set and the computer has it.
pub struct MockTools {
    pub compile: bool,
    pub made: std::sync::Mutex<Vec<PathBuf>>,
}

#[async_trait::async_trait]
impl Tools for MockTools {
    fn plot(&self, figs: &Path, stem: &str, spec: &lore_plot::Spec, _styles: &serde_json::Value) -> Result<String, String> {
        std::fs::create_dir_all(figs).map_err(|e| e.to_string())?;
        let saved = lore_plot::figure::save_files(figs, stem, &tiny_pdf(), &spec.to_json()).map_err(|e| e.to_string())?;
        self.made.lock().unwrap().push(figs.join(&saved.file));
        Ok(saved.file)
    }

    async fn compile(&self, book: &Path) -> Compiled {
        if self.compile { RealTools.compile(book).await } else { Compiled { skipped: Some("測試：沒有編譯".into()), ..Default::default() } }
    }
}
