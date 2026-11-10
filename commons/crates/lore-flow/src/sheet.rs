//! The module library at a glance: a contact sheet with one row per kind and
//! one cell per art style (built-in styles and saved drawings), and the
//! built-in styles written out as module files.

use lore_canvas::Object;

use crate::modules::{self, Ctx, Library, ModuleTemplate, SaveOptions};
use crate::spec::{MODULES, Node};
use crate::text::{MathSource, Vars};
use crate::theme::Theme;

/// A typical node of a kind, to show its drawing.
pub fn sample(kind: &str, style: &str) -> Node {
    let (label, params): (&str, serde_json::Value) = match kind {
        "box" => ("Model", serde_json::json!({})),
        "op" => ("+", serde_json::json!({})),
        "loss" => ("$\\mathcal L_{seg}$", serde_json::json!({})),
        "label" => ("Output", serde_json::json!({})),
        "encoder" => ("Enc", serde_json::json!({})),
        "decoder" => ("Dec", serde_json::json!({})),
        "autoencoder" => ("", serde_json::json!({})),
        "layer-bars" => ("", serde_json::json!({ "layers": ["Conv", "Pool", "Conv", "FC"] })),
        "cuboid-seq" => ("", serde_json::json!({ "shape": "hourglass", "n": 5 })),
        "nn" => ("", serde_json::json!({ "layers": [3, 4, 2] })),
        "image" | "image-stack" | "feature-stack" | "vector-bar" | "matrix" | "unet" | "brace" => ("", serde_json::json!({})),
        _ => ("Label", serde_json::json!({})),
    };
    let params = params.as_object().map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()).unwrap_or_default();
    Node { id: "m".into(), module: kind.into(), label: label.into(), style: style.into(), params, ..Default::default() }
}

/// Every (kind, style) that can be drawn: built-in kinds with their styles
/// (a kind without styles has one, `default`), then what the library adds.
pub fn catalogue(lib: &Library) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for (kind, _) in MODULES {
        let mut styles: Vec<String> = modules::generator_styles(kind).iter().map(|s| s.to_string()).collect();
        if styles.is_empty() {
            styles.push(modules::DEFAULT_STYLE.into());
        }
        for s in lib.styles(kind) {
            if !styles.contains(&s) {
                styles.push(s);
            }
        }
        out.push((kind.to_string(), styles));
    }
    for kind in lib.kinds() {
        out.push((kind.clone(), lib.styles(&kind)));
    }
    out
}

/// The contact sheet: objects and the page size in pt.
pub fn contact_sheet(theme: &Theme, lib: &Library, math: &dyn MathSource) -> (Vec<Object>, f64, f64) {
    let vars = Vars::default();
    let cat = catalogue(lib);
    let (cell_h, head_w, margin, per_line) = (104.0, 92.0, 18.0, 9usize);
    // the matched pair is twice as wide; long rows wrap
    let cell_w_of = |kind: &str| if kind == "autoencoder" { 208.0 } else { 104.0 };
    let per = |kind: &str| if kind == "autoencoder" { per_line / 2 } else { per_line };
    let mut lines: Vec<(&str, usize, &[String])> = Vec::new();
    for (kind, styles) in &cat {
        for (i, chunk) in styles.chunks(per(kind)).enumerate() {
            lines.push((kind.as_str(), i, chunk));
        }
    }
    let page_w = 2.0 * margin + head_w + per_line as f64 * 104.0;
    let page_h = 2.0 * margin + lines.len() as f64 * cell_h;
    let mut objs = Vec::new();
    let mut id = 2;
    let base = Ctx { theme, vars: &vars, math, lib, k: 1.0, symmetric: false, image_size: &modules::no_images };
    for (ri, (kind, part, styles)) in lines.iter().enumerate() {
        let (kind, part) = (*kind, *part);
        let y = margin + ri as f64 * cell_h;
        if part == 0 {
            let mut rule = modules::shape("", "rect", margin, y, page_w - 2.0 * margin, 0.0, lore_canvas::Paint::None, Some(lore_canvas::Stroke::new("D9D9D9", 0.5)));
            rule.id = id;
            rule.name = format!("sheet:{kind}/rule");
            id += 1;
            objs.push(rule);
            let (mut head, _, hh) = base.text("", kind, &base.ty(1.0, true, &theme.colors.ink), 0.0, 0.0);
            head.frame.x = modules::emu(margin);
            head.frame.y = modules::emu(y + (cell_h - hh) / 2.0);
            head.id = id;
            head.name = format!("sheet:{kind}/name");
            id += 1;
            objs.push(head);
        }
        let cell_w = cell_w_of(kind);
        for (ci, style) in styles.iter().enumerate() {
            let x = margin + head_w + ci as f64 * cell_w;
            let node = sample(kind, if style == modules::DEFAULT_STYLE && modules::generator_styles(kind).is_empty() && lib.get(kind, style).is_none() { "" } else { style });
            // as large as the cell allows, never more than 1.4 ×
            let natural = modules::make(&node, &base);
            let k = ((cell_w - 14.0) / natural.reserve_w.max(1.0)).min((cell_h - 26.0) / natural.h.max(1.0)).min(1.4);
            let cx = Ctx { theme, vars: &vars, math, lib, k, symmetric: false, image_size: &modules::no_images };
            let body = modules::make(&node, &cx);
            let (bw, bh) = (body.w, body.h);
            let (o, next) = modules::place(body, &format!("sheet:{kind}/{style}"), x + (cell_w - bw) / 2.0, y + 6.0 + (cell_h - 22.0 - bh) / 2.0, id);
            id = next;
            objs.push(o);
            let (mut cap, _, ch) = base.text("", style, &base.small(&theme.colors.muted), x + cell_w / 2.0, 0.0);
            cap.frame.y = modules::emu(y + cell_h - ch - 4.0);
            cap.id = id;
            cap.name = format!("sheet:{kind}/{style}/name");
            id += 1;
            objs.push(cap);
        }
    }
    (objs, page_w, page_h)
}

/// The built-in art styles as module drawings (what `modules/<kind>/<style>.json`
/// holds), so they can be copied into a library and reworked there. Colours
/// of layer types stay `layer:<type>`, the module's own colours `role:*`.
pub fn builtin_templates(theme: &Theme) -> Vec<ModuleTemplate> {
    let vars = Vars::default();
    let lib = Library::default();
    let cx = Ctx { theme, vars: &vars, math: &crate::text::NoMath, lib: &lib, k: 1.0, symmetric: false, image_size: &modules::no_images };
    let mut out = Vec::new();
    for (kind, description) in MODULES {
        if !matches!(kind, "encoder" | "decoder" | "autoencoder" | "nn" | "brace") {
            continue;
        }
        for style in modules::generator_styles(kind) {
            let mut node = sample(kind, style);
            let label = if node.label.is_empty() { None } else { Some(node.label.clone()) };
            node.role = "model".into();
            let body = modules::make(&node, &cx);
            let legend = body.legend.clone();
            let (o, _) = modules::place(body, "m", 0.0, 0.0, 2);
            // layer colours become placeholders before the role colours are picked
            let mut text = serde_json::to_string(&o).unwrap_or_default();
            // paper stays paper (the most used fill is the one that follows the role)
            text = text.replace(&format!("\"color\":\"{}\"", theme.colors.paper.to_uppercase()), "\"color\":\"theme:paper\"");
            for sw in &legend {
                text = text.replace(&format!("\"color\":\"{}\"", sw.color.to_uppercase()), &format!("\"color\":\"layer:{}\"", sw.key));
            }
            let Ok(o) = serde_json::from_str::<Object>(&text) else { continue };
            let opt = SaveOptions { kind: kind.to_string(), style: style.to_string(), description: description.to_string(), label, role_colours: true };
            if let Ok(t) = modules::template_from_objects(std::slice::from_ref(&o), &opt) {
                out.push(t);
            }
        }
    }
    out
}
