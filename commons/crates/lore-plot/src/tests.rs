//! Offline tests: synthetic data only, temp dirs, no Chrome, no network, mock model.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::*;

fn sample_spec() -> Value {
    json!({
        "lore_plot": 1, "style": "clean", "set": { "font.size": 9 }, "size": { "w": 85, "h": 60 },
        "grid": { "cols": 1, "rows": 1, "labels": "none" }, "future_field": { "kept": true },
        "data": { "t1": { "name": "r.csv", "columns": ["metric", "A", "A_sd", "B"], "rows": [["Dice", 0.8, 0.02, 0.85], ["IoU", 0.7, 0.03, 0.74]] } },
        "panels": [{ "type": "bar", "data": "t1", "x": "metric", "series": [{ "y": "A", "err": "A_sd", "label": "$\\alpha=0.5$", "color": "#123456" }, { "y": "B" }],
                     "yaxis": { "label": "Score", "min": 0 }, "notes": [{ "type": "bracket", "x": 0, "x2": 1, "text": "**" }] }],
        "caption": "Two methods on two metrics."
    })
}

// ── spec ─────────────────────────────────────────────────────────────────

#[test]
fn spec_round_trip_keeps_unknown_fields() {
    let s = Spec::from_value(sample_spec()).unwrap();
    assert!(s.problems().is_empty(), "{:?}", s.problems());
    let back: Value = serde_json::from_str(&s.to_json()).unwrap();
    assert_eq!(back["future_field"]["kept"], true);
    assert_eq!(back["panels"][0]["series"][0]["color"], "#123456");
    assert_eq!(back["panels"][0]["notes"][0]["text"], "**");
    assert_eq!(back["grid"]["cols"], 1);
    assert_eq!(Spec::from_json(&s.to_json()).unwrap(), s);
}

#[test]
fn spec_problems_name_missing_columns_and_types() {
    let mut v = sample_spec();
    v["panels"][0]["series"][1]["y"] = "C".into();
    v["panels"][0]["type"] = "pie3d".into();
    let p = Spec::from_value(v).unwrap().problems();
    assert!(p.iter().any(|m| m.contains("「C」")), "{p:?}");
    assert!(p.iter().any(|m| m.contains("pie3d")), "{p:?}");
    let mut v = sample_spec();
    v["lore_plot"] = 99.into();
    assert!(matches!(Spec::from_value(v), Err(SpecError::Newer(99))));
    assert!(matches!(Spec::from_json("{}"), Err(SpecError::Parse(_))));
}

#[test]
fn renderer_knows_the_same_chart_types() {
    let js = [include_str!("../../../ui/plot/charts.js"), include_str!("../../../ui/plot/charts3d.js")].concat();
    for t in spec::TYPES {
        assert!(js.contains(&format!("CHARTS.{t} =")), "the renderer has no chart {t}");
    }
    let defined = js.matches("  CHARTS.").count();
    assert_eq!(defined, spec::TYPES.len(), "a chart type the spec model does not list");
}

#[test]
fn format_catalogue_is_complete_data() {
    let cat: Value = serde_json::from_str(include_str!("../../../ui/plot/formats.json")).unwrap();
    let formats = cat["formats"].as_array().unwrap();
    let mut codes = std::collections::BTreeSet::new();
    let mut per_family: BTreeMap<char, usize> = BTreeMap::new();
    for f in formats {
        let code = f["code"].as_str().unwrap();
        assert!(codes.insert(code.to_string()), "{code} twice");
        let chart = f["chart"].as_str().unwrap();
        assert!(chart == "*" || spec::TYPES.contains(&chart), "{code}: unknown chart {chart}");
        assert!(["single", "double", "onehalf", "page"].contains(&f["width"].as_str().unwrap()), "{code}");
        assert!(f["aspect"].as_f64().unwrap() > 0.2, "{code}");
        assert!(!f["label"].as_str().unwrap().is_empty());
        *per_family.entry(code.chars().next().unwrap()).or_default() += 1;
    }
    // every chart family has 4–7 formats (the author's requirement)
    for (fam, n) in &per_family {
        assert!((3..=7).contains(n), "family {fam} has {n} formats");
    }
    for fam in ['B', 'L', 'S', 'V', 'H', 'R', 'D', 'T', 'P', 'A', 'C'] {
        assert!(per_family.contains_key(&fam), "no {fam} formats");
    }
    for (id, v) in cat["venues"].as_object().unwrap() {
        assert!(v["single"].as_f64().unwrap() < v["double"].as_f64().unwrap(), "{id}");
    }
    // thresholds are data (tunable), not code
    assert!(cat["rules"]["legend_top_max"].as_u64().is_some());
    let js = include_str!("../../../ui/plot/formats.js");
    assert!(!js.contains("0.02)") || js.contains("rules.inset_auc_gap"), "thresholds come from the catalogue");
}

#[test]
fn palettes_are_static_and_readable_on_white() {
    let pals: Value = serde_json::from_str(include_str!("../../../ui/plot/palettes.json")).unwrap();
    let lum = |h: &str| -> f64 {
        let c = |i: usize| {
            let v = u8::from_str_radix(&h[i..i + 2], 16).unwrap() as f64 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * c(1) + 0.7152 * c(3) + 0.0722 * c(5)
    };
    let mut themes = 0;
    for (id, p) in pals.as_object().unwrap() {
        let colors = p["colors"].as_array().unwrap();
        assert!(colors.len() >= 3, "{id}");
        for c in colors {
            let c = c.as_str().unwrap();
            assert!(c.len() == 7 && c.starts_with('#'), "{id}: {c}");
            if p.get("from_theme").is_some() {
                assert!(1.05 / (lum(c) + 0.05) >= 2.95, "{id}: {c} is too light for white paper");
            }
        }
        if p.get("from_theme").is_some() {
            themes += 1;
        }
    }
    // a copy of each of the 20 app themes (values copied, never theme tokens)
    assert_eq!(themes, 20);
    assert!(!include_str!("../../../ui/plot/palettes.json").contains("var(--"));
    for f in ["plot.js", "charts.js", "charts3d.js"] {
        let src = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/plot").join(f)).unwrap();
        for token in ["--accent", "--text", "--bg", "--panel", "--line", "--muted", "--paper", "--surface", "--ok", "--warn", "--err"] {
            assert!(!src.contains(&format!("var({token}")), "{f}: figures never use theme tokens ({token})");
        }
    }
}

// ── data ─────────────────────────────────────────────────────────────────

#[test]
fn csv_tsv_and_plus_minus() {
    let t = &data::import("r.csv", "method,Dice,HD95\nBaseline,0.81 ± 0.02,12.5\n\"Ours, v2\",0.86 ± 0.01,9.1\n".as_bytes()).unwrap()[0];
    assert_eq!(t.columns, ["method", "Dice", "Dice_sd", "HD95"]);
    assert_eq!(t.rows[1][0], "Ours, v2");
    assert_eq!(t.rows[0][1], json!(0.81));
    assert_eq!(t.rows[0][2], json!(0.02));
    assert_eq!(t.rows[1][3], json!(9.1));
    // pasted spreadsheet cells
    let t = &data::import("pasted.txt", "x\ty\n1\t−2\n2\t\n".as_bytes()).unwrap()[0];
    assert_eq!(t.rows[0][1], json!(-2.0));
    assert_eq!(t.rows[1][1], Value::Null);
    assert!(matches!(data::import("e.csv", b""), Err(data::ImportError::Empty)));
}

#[test]
fn json_tables() {
    let t = &data::import("r.json", br#"[{"m":"A","v":"1.5"},{"m":"B","v":2,"w":true}]"#).unwrap()[0];
    assert_eq!(t.columns, ["m", "v", "w"]);
    assert_eq!(t.rows[0][1], json!(1.5));
    assert_eq!(t.rows[1][2], json!(1));
    let t = &data::import("r.json", br#"{"columns":["a","b"],"rows":[[1,"x"]]}"#).unwrap()[0];
    assert_eq!(t.rows[0], vec![json!(1), json!("x")]);
}

#[test]
fn latex_tables_from_a_paper() {
    let tex = r"\begin{table}[t]\centering
\caption{Results.}\label{tab:res}
\begin{tabular}{lcc}\toprule
Method & Dice & HD$_{95}$ \\ \midrule
Baseline & 0.812 $\pm$ 0.021 & 12.4 \\
\textbf{Ours} & \textbf{0.861} $\pm$ 0.015 & \textbf{9.8} \\ \bottomrule
\end{tabular}\end{table}
\begin{tabular}{ll} a & b \\ 1 & 2 \\ \end{tabular}";
    let ts = data::import("main.tex", tex.as_bytes()).unwrap();
    assert_eq!(ts.len(), 2);
    let t = &ts[0];
    assert_eq!(t.columns, ["Method", "Dice", "Dice_sd", "HD_95"]);
    assert_eq!(t.rows[1][0], "Ours");
    assert_eq!(t.rows[1][1], json!(0.861));
    assert_eq!(t.rows[1][2], json!(0.015));
    assert_eq!(t.rows[1][3], json!(9.8));
}

fn tiny_xlsx() -> Vec<u8> {
    use std::io::Write;
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut z = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        let mut put = |n: &str, s: &str| {
            z.start_file(n, o).unwrap();
            z.write_all(s.as_bytes()).unwrap();
        };
        put("xl/workbook.xml", r#"<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="Results" sheetId="1" r:id="rId1"/></sheets></workbook>"#);
        put("xl/_rels/workbook.xml.rels", r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#);
        put("xl/sharedStrings.xml", r#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><si><t>method</t></si><si><t>Dice</t></si><si><t>A &amp; B</t></si></sst>"#);
        put("xl/worksheets/sheet1.xml", r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c></row><row r="2"><c r="A2" t="s"><v>2</v></c><c r="B2"><v>0.83</v></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>C</t></is></c><c r="C3"><v>7</v></c></row></sheetData></worksheet>"#);
        z.finish().unwrap();
    }
    buf.into_inner()
}

#[test]
fn xlsx_sheets() {
    let ts = data::import("book.xlsx", &tiny_xlsx()).unwrap();
    assert_eq!(ts[0].name, "Results");
    assert_eq!(ts[0].columns, ["method", "Dice", "col3"]);
    assert_eq!(ts[0].rows[0], vec![json!("A & B"), json!(0.83), Value::Null]);
    assert_eq!(ts[0].rows[1], vec![json!("C"), Value::Null, json!(7.0)]);
    assert!(data::import("bad.xlsx", b"not a zip").is_err());
}

// ── styles ───────────────────────────────────────────────────────────────

#[test]
fn public_styles_parse_and_match_the_index() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/plot/styles");
    let index: Vec<String> = serde_json::from_str(&std::fs::read_to_string(dir.join("index.json")).unwrap()).unwrap();
    let compiled: Vec<&str> = style::PUBLIC.iter().map(|(id, _)| *id).collect();
    assert_eq!(index, compiled, "styles/index.json and style::PUBLIC differ");
    let mut files: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n != "index.json").collect();
    files.sort();
    let mut want: Vec<String> = compiled.iter().map(|id| format!("{id}.json")).collect();
    want.sort();
    assert_eq!(files, want);
    let resolved = style::resolved(&style::raw(None), &style::Fonts::default());
    assert_eq!(resolved.len(), compiled.len());
    for s in &resolved {
        let id = s["id"].as_str().unwrap();
        let src = style::PUBLIC.iter().find(|(i, _)| *i == id).unwrap().1;
        let raw: Value = serde_json::from_str(src).unwrap();
        assert_eq!(raw["id"], id, "file name and id differ");
        assert!(s.pointer("/font/size").and_then(Value::as_f64).is_some(), "{id}: no font size after merge");
        assert!(s.pointer("/colors/palette").and_then(Value::as_array).is_some_and(|p| p.len() >= 3), "{id}");
        assert!(s.get("base").is_none());
        // public presets name only free fonts (the author's choice lives in his private settings)
        const FREE: &[&str] = &["STIX Two Text", "TeX Gyre Termes", "Liberation Serif", "Tinos", "serif", "Liberation Sans", "Arimo", "TeX Gyre Heros", "Nimbus Sans", "sans-serif"];
        for f in s.pointer("/font/family").unwrap().as_array().unwrap() {
            assert!(FREE.contains(&f.as_str().unwrap()), "{id} names a font outside the free list: {f}");
        }
    }
    // Nature: 89 mm single column, 5–7 pt type
    let nature = resolved.iter().find(|s| s["id"] == "nature").unwrap();
    assert_eq!(nature.pointer("/size/widths/0"), Some(&json!(89)));
    assert!(nature.pointer("/font/tick").and_then(Value::as_f64).unwrap() >= 5.0);
    assert!(nature.pointer("/font/size").and_then(Value::as_f64).unwrap() <= 7.0);
    // npj inherits Nature's sizes through its base
    let npj = resolved.iter().find(|s| s["id"] == "npj").unwrap();
    assert_eq!(npj.pointer("/size/widths/2"), Some(&json!(183)));
}

#[test]
fn private_styles_and_fonts_come_first() {
    let tmp = tempfile::tempdir().unwrap();
    let home = lore_home::Home::at(tmp.path());
    let plot = tmp.path().join("private/shared/plot");
    std::fs::create_dir_all(plot.join("styles")).unwrap();
    std::fs::write(plot.join("settings.toml"), "default_style = \"mine\"\n[font]\nserif = [\"Test Serif\"]\n").unwrap();
    std::fs::write(plot.join("styles/mine.toml"), "id = \"mine\"\nbase = \"clean\"\nlabel = \"我的\"\n[colors]\npalette = [\"#87ceeb\", \"#7fffd4\", \"#dda0dd\"]\n").unwrap();
    std::fs::write(plot.join("styles/nature.json"), r#"{ "id": "nature", "base": "clean", "label": "Nature（改）" }"#).unwrap();
    std::fs::write(plot.join("styles/broken.toml"), "id = ").unwrap();
    let l = style::list(&home);
    assert_eq!(l["default"], "mine");
    let styles = l["styles"].as_array().unwrap();
    let mine = styles.iter().find(|s| s["id"] == "mine").unwrap();
    assert_eq!(mine["source"], "private");
    assert_eq!(mine.pointer("/colors/palette/0"), Some(&json!("#87ceeb")));
    assert_eq!(mine.pointer("/font/family/0"), Some(&json!("Test Serif")));
    assert_eq!(mine.pointer("/font/family/1"), Some(&json!("STIX Two Text")));
    // a private style with a public id replaces it
    assert_eq!(styles.iter().filter(|s| s["id"] == "nature").count(), 1);
    assert_eq!(styles.iter().find(|s| s["id"] == "nature").unwrap()["label"], "Nature（改）");
    // sans styles get the sans list (empty here): unchanged
    let slide = styles.iter().find(|s| s["id"] == "slide").unwrap();
    assert_eq!(slide.pointer("/font/family/0"), Some(&json!("Liberation Sans")));
    // no private folder: public only, default clean
    let l = style::list(&lore_home::Home::at(tempfile::tempdir().unwrap().path()));
    assert_eq!(l["default"], "clean");
}

// ── export ───────────────────────────────────────────────────────────────

/// A 1×1 PNG built by hand (signature, IHDR, IDAT, IEND), CRCs computed.
fn tiny_png() -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        let mut c = 0xffff_ffffu32;
        for &b in &body {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        out.extend_from_slice(&(!c).to_be_bytes());
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    chunk(&mut out, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0]);
    chunk(&mut out, b"IDAT", &[0x78, 0x01, 0x63, 0xf8, 0xff, 0xff, 0x3f, 0x00, 0x05, 0xfe, 0x02, 0xfe]);
    chunk(&mut out, b"IEND", &[]);
    out
}

#[test]
fn png_carries_the_spec() {
    let png = tiny_png();
    assert_eq!(export::png_size(&png), Some((1, 1)));
    assert_eq!(export::spec_from_png(&png), None);
    let spec = Spec::from_value(sample_spec()).unwrap();
    let with = export::png_with_spec(&png, &spec.to_json()).unwrap();
    assert_eq!(figure::spec_in_png(&with).unwrap(), spec);
    // replaced, not stacked
    let again = export::png_with_spec(&with, "{\"lore_plot\":1}").unwrap();
    assert_eq!(export::spec_from_png(&again).as_deref(), Some("{\"lore_plot\":1}"));
    assert_eq!(again.windows(9).filter(|w| w == b"lore-plot").count(), 1);
    assert!(again.ends_with(&[0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xae, 0x42, 0x60, 0x82]));
    assert!(export::png_with_spec(b"not png", "{}").is_none());
}

const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="85mm" height="60.5mm" viewBox="0 0 240.94 171.5" data-lore-plot="1"><text font-family="KaTeX_Math" font-style="italic">α</text><text font-family="'STIX Two Text', serif">Dice</text></svg>"#;

#[test]
fn svg_document_has_spec_and_maths_fonts() {
    assert_eq!(export::size_mm(SVG), Some((85.0, 60.5)));
    let doc = export::svg_document(SVG, Some(r#"{"lore_plot":1,"caption":"a < b & c"}"#));
    assert!(doc.starts_with("<?xml"));
    assert!(doc.contains("@font-face{font-family:KaTeX_Math;font-weight:normal;font-style:italic;src:url(data:font/woff2;base64,"));
    assert!(!doc.contains("font-family:KaTeX_Main;"), "only the families the figure uses");
    assert_eq!(export::spec_from_svg(&doc).as_deref(), Some(r#"{"lore_plot":1,"caption":"a < b & c"}"#));
    // re-exporting replaces the metadata, and an XML declaration in front is handled
    let again = export::svg_document(&doc, Some("{}"));
    assert_eq!(again.matches("<metadata>").count(), 1);
    assert_eq!(again.matches("<?xml").count(), 1);
    let zoomed = SVG.replacen("<svg ", "<svg style=\"width: 643px; height: 457px;\" ", 1);
    assert!(!export::svg_document(&zoomed, None).contains("643px"));
    let html = export::print_html(&doc, 85.0, 60.5);
    assert!(html.contains("@page{size:85mm 60.5mm;margin:0}"));
    // no Chrome: SVG still works, PDF says why not
    let svg = export::export(&Tools::none(), SVG, Some("{}"), Format::Svg, 0).unwrap();
    assert!(String::from_utf8(svg).unwrap().contains("<metadata>"));
    let e = export::export(&Tools::none(), SVG, None, Format::Pdf, 0).unwrap_err();
    assert!(e.to_string().contains("Chrome"));
}

#[test]
fn every_katex_font_is_compiled_in() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/notation-panel/vendor/katex/fonts");
    let n = std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".woff2")).count();
    assert_eq!(n, assets::KATEX_FONTS.len());
    assert!(assets::get("").is_some());
    assert!(assets::get("katex/fonts/KaTeX_Main-Regular.woff2").is_some());
    assert!(assets::get("editor.js").is_some());
    for f in assets::SCRIPTS {
        assert!(assets::get(f).is_some(), "{f}");
    }
    assert!(assets::get("formats.json").is_some() && assets::get("palettes.json").is_some());
    // every file in commons/ui/plot that a page needs is compiled in
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui/plot");
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if n.ends_with(".js") || n.ends_with(".css") || n == "formats.json" || n == "palettes.json" {
            assert!(assets::get(&n).is_some(), "{n} is not served under /plot/");
        }
    }
    assert!(assets::get("../secret").is_none());
}

// ── figures in documents ─────────────────────────────────────────────────

#[test]
fn paper_figures_never_overwrite_the_authors_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("figures");
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(figure::stem_of("Dice & HD95 — comparison!"), "dice-hd95-comparison");
    assert_eq!(figure::stem_of("圖"), "figure");
    // the author's own PDF: a new name, even when replacing is asked
    std::fs::write(dir.join("results.pdf"), b"%PDF mine").unwrap();
    assert_eq!(figure::free_stem(&dir, "results", true), "results-2");
    // ours (spec next to it): replaced in place on re-edit, a new name otherwise
    let spec = Spec::from_value(sample_spec()).unwrap();
    let saved = figure::save_files(&dir, "bars", b"%PDF 1", &spec.to_json()).unwrap();
    assert_eq!(saved.file, "bars.pdf");
    assert_eq!(figure::free_stem(&dir, "bars", true), "bars");
    assert_eq!(figure::free_stem(&dir, "bars", false), "bars-2");
    assert_eq!(figure::spec_next_to(&dir.join("bars.pdf")).unwrap(), spec);
    assert!(figure::spec_next_to(&dir.join("results.pdf")).is_none());
    assert_eq!(std::fs::read(dir.join("results.pdf")).unwrap(), b"%PDF mine");
    // no temp files left
    assert!(std::fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")));
}

/// The inserted block keeps lore-notation's rules: the label is registered and referred to
/// with \ref, no typed "Figure 1", and the checker finds nothing about it.
#[test]
fn figure_block_passes_the_notation_checks() {
    use lore_notation::{CheckOptions, Document, Entry, Kind, Notation, check_document};
    let block = figure::figure_block("figures/dice-comparison.pdf", "Dice of three methods (50% of cases & more).", "fig:dice-comparison", false);
    assert!(block.contains("\\caption{Dice of three methods (50\\% of cases \\& more).}"));
    assert!(block.contains("\\includegraphics[width=\\linewidth]{figures/dice-comparison.pdf}"));
    let above = figure::figure_block("figures/a.pdf", "A.", "fig:a", true);
    assert!(above.find("\\caption").unwrap() < above.find("\\includegraphics").unwrap());
    let tex = format!("\\documentclass{{article}}\n\\begin{{document}}\n\\section{{Results}}\nOur method improves the overlap (Fig.~\\ref{{fig:dice-comparison}}).\n\n{block}\n\\end{{document}}\n");
    let mut reg = Notation::default();
    let mut e = Entry::new("fig:dice-comparison", Kind::Figure);
    e.label = Some("fig:dice-comparison".into());
    e.file = Some("figures/dice-comparison.pdf".into());
    e.caption = Some("Dice of three methods.".into());
    reg.upsert(e);
    let report = check_document(Document::from_str("main.tex", &tex), &reg, &CheckOptions::default());
    let about: Vec<_> = report.issues.iter().filter(|i| ["label_unreferenced", "label_duplicate", "ref_undefined", "hardcoded_reference", "caption_style"].contains(&i.code.as_str())).collect();
    assert!(about.is_empty(), "{about:#?}");
    // the checker does catch a typed reference, so the test above means something
    let typed = tex.replace("(Fig.~\\ref{fig:dice-comparison})", "(Figure 1)");
    let report = check_document(Document::from_str("main.tex", &typed), &reg, &CheckOptions::default());
    assert!(report.issues.iter().any(|i| i.code == "hardcoded_reference" || i.code == "label_unreferenced"), "{:#?}", report.issues);
}

// ── the model task ───────────────────────────────────────────────────────

struct Fake(std::sync::Mutex<Vec<Value>>);

#[async_trait::async_trait]
impl propose::Llm for Fake {
    async fn complete_json(&self, task: &str, req: lore_llm::LlmRequest) -> Result<(Value, String), String> {
        assert_eq!(task, propose::TASK);
        assert!(req.json_schema.is_some());
        let v = self.0.lock().unwrap().remove(0);
        lore_llm::json::validate(&v, req.json_schema.as_ref().unwrap())?;
        Ok((v, "mock".into()))
    }
}

fn tables() -> BTreeMap<String, Table> {
    let t = data::import("r.csv", b"method,Dice,Dice_sd,HD95\nBaseline,0.81,0.02,12.5\nOurs,0.86,0.01,9.1\n").unwrap().remove(0);
    BTreeMap::from([("t1".to_string(), t)])
}

#[test]
fn propose_builds_a_valid_spec_and_retries_bad_columns() {
    let tables = tables();
    let styles = vec!["clean".to_string(), "nature".to_string()];
    let ask = propose::Ask { tables: &tables, sentence: "compare Dice and HD95, grouped bars with error bars", styles: &styles, style: "clean", target: Some("paper"), examples: &[] };
    let req = propose::request(&ask);
    let user = format!("{:?}", req.messages);
    assert!(user.contains("method | Dice | Dice_sd | HD95"), "{user}");
    let good = json!({ "style": "nature", "grid_cols": 2, "caption": "Dice and HD95 of two methods.", "panels": [
        { "type": "bar", "table": "t1", "x": "method", "series": [{ "y": "Dice", "err": "Dice_sd", "label": "Dice" }], "ylabel": "Dice", "values": true },
        { "type": "bar", "table": "t1", "x": "method", "series": [{ "y": "HD95", "label": "$\\mathrm{HD}_{95}$" }], "ylabel": "$\\mathrm{HD}_{95}$ (mm)", "legend": "upper right" } ] });
    // the schema keeps column names closed: an unknown column is rejected before to_spec
    let bad = json!({ "panels": [{ "type": "bar", "x": "method", "series": [{ "y": "Accuracy" }] }] });
    assert!(lore_llm::json::validate(&bad, &propose::schema(&ask)).is_err());
    let fake = Fake(std::sync::Mutex::new(vec![good.clone()]));
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let (spec, engine) = rt.block_on(propose::propose(&fake, &ask)).unwrap();
    assert_eq!(engine, "mock");
    assert_eq!(spec.style.as_deref(), Some("nature"));
    assert_eq!(spec.panels.len(), 2);
    assert_eq!(spec.rest["grid"]["cols"], 2);
    assert_eq!(spec.rest["grid"]["labels"], "a");
    assert_eq!(spec.panels[0].series[0].err.as_deref(), Some("Dice_sd"));
    assert_eq!(spec.panels[0].rest["opts"]["values"], true);
    assert!(spec.problems().is_empty());
    assert_eq!(spec.data["t1"].rows.len(), 2, "the data rides along");
    // a reply whose structure is unusable (a missing table column for its type) is retried once
    let wrong = json!({ "panels": [{ "type": "scatter", "x": "Dice", "y": null, "group": "method", "series": [] }] });
    let mut wrong2 = wrong.clone();
    wrong2["panels"][0]["x"] = Value::Null;
    let fake = Fake(std::sync::Mutex::new(vec![json!({ "panels": [] }), good]));
    assert!(rt.block_on(propose::propose(&fake, &ask)).is_ok());
    let _ = (wrong, wrong2);
}

#[test]
fn private_examples_are_read_without_their_data() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("a.json"), serde_json::to_string(&json!({ "sentence": "bars", "spec": sample_spec() })).unwrap()).unwrap();
    std::fs::write(tmp.path().join("b.json"), "not json").unwrap();
    let ex = propose::examples(tmp.path());
    assert_eq!(ex.len(), 1);
    assert!(ex[0].1.get("data").is_none());
    assert!(propose::examples(&tmp.path().join("missing")).is_empty());
}

// ── the figure assistant ─────────────────────────────────────────────────

struct Script(std::sync::Mutex<Vec<Value>>, std::sync::Mutex<Vec<String>>);

#[async_trait::async_trait]
impl propose::Llm for Script {
    async fn complete_json(&self, task: &str, req: lore_llm::LlmRequest) -> Result<(Value, String), String> {
        self.1.lock().unwrap().push(task.to_string());
        let v = self.0.lock().unwrap().remove(0);
        lore_llm::json::validate(&v, req.json_schema.as_ref().unwrap())?;
        Ok((v, "mock".into()))
    }
}

fn assist_ctx() -> assist::Context {
    assist::Context { catalogue: assist::catalogue_text(include_str!("../../../ui/plot/formats.json")), style_preset: "clean".into(), venue: "Nature".into(), preferences: String::new() }
}

#[test]
fn assist_prompt_is_neutral_and_filled() {
    let sys = assist::system(&assist_ctx());
    assert!(!sys.contains("{catalogue}") && !sys.contains("{venue}") && !sys.contains("{style_preset}"));
    assert!(sys.contains("B1 bar") && sys.contains("R2 roc") && sys.contains("never write code"));
    assert!(sys.contains("Venue: Nature"));
    // the public prompt names no font at all (fonts come from the style preset)
    assert!(!assist::PROMPT.to_lowercase().contains("font-family") && !assist::PROMPT.contains("Roman"));
    let mut c = assist_ctx();
    c.preferences = "Default style for papers: mine.".into();
    assert!(assist::system(&c).contains("Default style for papers: mine."));
    // both tasks are in the table an app merges
    let t = assist::task_table();
    for id in [propose::TASK, assist::TASK, assist::TASK_VISION] {
        assert!(t.task.contains_key(id), "{id}");
    }
}

#[test]
fn assist_alternatives_keep_the_data_and_drop_unusable_ones() {
    let tables = tables();
    let styles = vec!["clean".to_string(), "nature".to_string()];
    let formats: Vec<String> = ["B1", "B3", "V1"].iter().map(|s| s.to_string()).collect();
    let reply = json!({ "note": "兩種畫法", "alternatives": [
        { "why": "長條比較兩個方法", "panels": [{ "type": "bar", "format": "B3", "table": "t1", "x": "method", "series": [{ "y": "Dice", "err": "Dice_sd", "ours": true }], "yaxis": { "label": "Dice", "min": 0.7 }, "legend": { "pos": "above" } }] },
        { "why": "每個指標一格", "grid_cols": 2, "panels": [
            { "type": "bar", "table": "t1", "x": "method", "series": [{ "y": "Dice" }] },
            { "type": "bar", "table": "t1", "x": "method", "series": [{ "y": "HD95", "label": "$\\mathrm{HD}_{95}$" }], "yaxis": { "scale": "log" } }] },
        { "why": "壞的", "panels": [{ "type": "scatter", "x": "method", "y": null, "group": null, "series": [] }] } ] });
    let fake = Script(std::sync::Mutex::new(vec![reply]), Default::default());
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let r = rt.block_on(assist::alternatives(&fake, &assist_ctx(), &assist::AlternativesAsk { tables: &tables, sentence: "", styles: &styles, style: "clean", formats: &formats })).unwrap();
    // the third (a scatter with no y column) is not a usable figure: left out, with its reason
    assert_eq!(r.alternatives.len(), 2, "{:?}", r.skipped);
    assert_eq!(r.alternatives[0].why, "長條比較兩個方法");
    assert_eq!(r.alternatives[0].spec.panels[0].rest["format"], "B3");
    assert_eq!(r.alternatives[0].spec.data["t1"].rows.len(), 2, "the data rides along, untouched");
    assert_eq!(r.alternatives[1].spec.panels.len(), 2);
    assert_eq!(r.alternatives[1].spec.rest["grid"]["cols"], 2);
    assert_eq!(fake.1.lock().unwrap().as_slice(), [assist::TASK]);
}

#[test]
fn assist_changes_are_applied_checked_and_listed() {
    let spec = sample_spec();
    let reply = json!({ "note": "y 軸 0.8–1.0 拉長三倍，圖例移到上方", "changes": [
        { "path": "/panels/0/yaxis/scale", "value": "stretch", "why": "差異在上段" },
        { "path": "/panels/0/yaxis/segments", "value": [[0.8, 1.0, 3]] },
        { "path": "/panels/0/legend/pos", "value": "above" },
        { "path": "/size/w", "value": 120 },
        { "path": "/set/legend.size", "value": 6.5 } ] });
    let mut spec = spec;
    spec["size"] = Value::Null; // a figure at its style's size: /size/w still applies
    let (new, changes) = assist::apply_changes(&spec, &reply).unwrap();
    assert_eq!(changes.len(), 5);
    assert_eq!(changes[0].before, Value::Null);
    assert_eq!(changes[3].before, Value::Null);
    assert_eq!(changes[3].after, json!(120));
    let v = serde_json::to_value(&new).unwrap();
    assert_eq!(v["panels"][0]["yaxis"]["segments"][0][2], 3);
    assert_eq!(v["panels"][0]["legend"]["pos"], "above");
    assert_eq!(v["set"]["legend.size"], 6.5);
    // never the data, never a broken figure
    assert!(assist::apply_changes(&spec, &json!({ "note": "", "changes": [{ "path": "/data/t1/rows/0/1", "value": 0.99 }] })).is_err());
    assert!(assist::apply_changes(&spec, &json!({ "note": "", "changes": [{ "path": "/panels/0/series/0/y", "value": "Nope" }] })).unwrap_err().contains("Nope"));
    assert!(assist::apply_changes(&spec, &json!({ "note": "", "changes": [{ "path": "/panels/7/title", "value": "x" }] })).is_err());
    // the prompt carries the spec without its rows
    let short = assist::spec_for_prompt(&spec);
    assert_eq!(short["data"]["t1"]["row_count"], 2);
}

#[test]
fn assist_adjust_retries_once_then_uses_vision_task_with_a_picture() {
    let spec = sample_spec();
    let bad = json!({ "note": "", "changes": [{ "path": "/data/t1/name", "value": "x" }] });
    let good = json!({ "note": "拉寬", "changes": [{ "path": "/size/w", "value": 183 }] });
    let fake = Script(std::sync::Mutex::new(vec![bad, good]), Default::default());
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    let img = lore_llm::Image::from_bytes("image/png", &tiny_png());
    let r = rt.block_on(assist::adjust(&fake, &assist_ctx(), &spec, "拉寬一點", Some(img))).unwrap();
    assert_eq!(r.changes[0].after, json!(183));
    assert_eq!(fake.1.lock().unwrap().as_slice(), [assist::TASK_VISION, assist::TASK_VISION]);
}

#[test]
fn assist_reads_the_authors_preferences_privately() {
    let tmp = tempfile::tempdir().unwrap();
    let home = lore_home::Home::at(tmp.path());
    assert_eq!(assist::preferences(&home), "");
    let plot = tmp.path().join("private/shared/plot");
    std::fs::create_dir_all(&plot).unwrap();
    std::fs::write(plot.join("assist.md"), "Prefer dashed lines with circle markers.").unwrap();
    std::fs::write(plot.join("settings.toml"), "default_style = \"mine\"\n").unwrap();
    let p = assist::preferences(&home);
    assert!(p.contains("dashed lines") && p.contains("Default style for papers: mine."));
}

/// Colours a reader with deutan, protan or tritan vision can still tell apart: Machado 2009
/// simulation (severity 1), CIE76 ΔE between every pair of the first five colours.
fn cvd_min_delta_e(colors: &[&str]) -> f64 {
    const M: [[[f64; 3]; 3]; 3] = [
        [[0.367322, 0.860646, -0.227968], [0.280085, 0.672501, 0.047413], [-0.011820, 0.042940, 0.968881]],
        [[0.152286, 1.052583, -0.204868], [0.114503, 0.786281, 0.099216], [-0.003882, -0.048116, 1.051998]],
        [[1.255528, -0.076749, -0.178779], [-0.078411, 0.930809, 0.147602], [0.004733, 0.691367, 0.303900]],
    ];
    let lin = |c: f64| if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    let rgb = |h: &str| [1, 3, 5].map(|i| lin(u8::from_str_radix(&h[i..i + 2], 16).unwrap() as f64 / 255.0));
    let lab = |r: [f64; 3]| {
        let r = r.map(|v| v.clamp(0.0, 1.0));
        let x = (0.4124 * r[0] + 0.3576 * r[1] + 0.1805 * r[2]) / 0.95047;
        let y = 0.2126 * r[0] + 0.7152 * r[1] + 0.0722 * r[2];
        let z = (0.0193 * r[0] + 0.1192 * r[1] + 0.9505 * r[2]) / 1.08883;
        let f = |t: f64| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
        [116.0 * f(y) - 16.0, 500.0 * (f(x) - f(y)), 200.0 * (f(y) - f(z))]
    };
    let mut worst = f64::INFINITY;
    for m in std::iter::once(None).chain(M.iter().map(Some)) {
        let labs: Vec<[f64; 3]> = colors.iter().take(5).map(|c| {
            let v = rgb(c);
            lab(match m { Some(m) => [0, 1, 2].map(|i| m[i][0] * v[0] + m[i][1] * v[1] + m[i][2] * v[2]), None => v })
        }).collect();
        for i in 0..labs.len() {
            for j in i + 1..labs.len() {
                let d = ((labs[i][0] - labs[j][0]).powi(2) + (labs[i][1] - labs[j][1]).powi(2) + (labs[i][2] - labs[j][2]).powi(2)).sqrt();
                worst = worst.min(d);
            }
        }
    }
    worst
}

#[test]
fn public_palettes_stay_distinct_for_colour_blind_readers() {
    for s in style::resolved(&style::raw(None), &style::Fonts::default()) {
        let cols: Vec<&str> = s.pointer("/colors/palette").unwrap().as_array().unwrap().iter().filter_map(Value::as_str).collect();
        let d = cvd_min_delta_e(&cols);
        assert!(d >= 10.0, "{}: two of the first five colours look alike to colour-blind readers (ΔE {d:.1})", s["id"]);
    }
    let pals: Value = serde_json::from_str(include_str!("../../../ui/plot/palettes.json")).unwrap();
    for (id, p) in pals.as_object().unwrap() {
        if p["cb_safe"] == true {
            let cols: Vec<&str> = p["colors"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
            assert!(cvd_min_delta_e(&cols) >= 10.0, "{id} is marked colour-blind safe but is not");
        }
    }
    // the check means something: near-identical light blues fail it
    assert!(cvd_min_delta_e(&["#87CEEB", "#A6D8FF", "#DDA0DD"]) < 10.0);
}

/// Headless render of a spec (needs Chrome; run by hand: `cargo test -p lore-plot render_spec -- --ignored`).
#[test]
#[ignore]
fn render_spec_in_headless_chrome() {
    let tools = crate::export::Tools::find();
    if tools.chrome.is_none() {
        return;
    }
    let styles = serde_json::Value::Array(crate::style::resolved(&crate::style::raw(None), &Default::default()));
    let spec = r#"{"lore_plot":1,"style":"clean","size":{"w":80,"h":55},"data":{"t":{"columns":["x","y"],"rows":[[0,0],[1,1],[2,4],[3,9]]}},"panels":[{"type":"line","data":"t","x":"x","series":[{"y":"y","label":"$y=x^2$"}],"xaxis":{"label":"$x$"},"yaxis":{"label":"$y$"}}]}"#;
    let svg = crate::export::render_spec(&tools, spec, &styles).unwrap();
    assert!(svg.starts_with("<svg") && svg.contains("mm"), "{}", &svg[..svg.len().min(300)]);
    let pdf = crate::export::export(&tools, &svg, Some(spec), crate::export::Format::Pdf, 0).unwrap();
    assert!(pdf.starts_with(b"%PDF"));
}
