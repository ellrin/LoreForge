//! 流程圖 on a slide (lore-flow's shared editor, `docs/flow-figures.md`).
//!
//! | Method | Path | |
//! |---|---|---|
//! | * | /flow/… | `lore_flow::web::router`: the editor's files, the steps, layouts, previews, 助手 (`doc` = the deck's name → its notation) |
//! | POST | /api/decks/{name}/flow | `{slide, spec, replace?}` → `{ops}`: the figure as one picture (a group that keeps its aspect ratio and carries the spec), inserted or put in the place of `replace`; the page commits the ops (one undo step). New variables join the deck's `notation.json`. |
//! | POST | /api/decks/{name}/image | `{slide, id, format: png｜svg｜pdf}` → `{path}`: one object or group saved as a picture in the deck's `exports/` |
//!
//! The spec rides on the group (`GroupObj::data`): in `deck.json`, and in the .pptx's editor-state
//! part, so a figure still re-opens after the file is saved and opened again.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use lore_canvas::{Kind, Object, Op};
use lore_flow::figure::{Catalogue, FlowSpec, as_picture, render};
use lore_flow::layout::Rect;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{R, Shared, bad, global_notation, notation, session};

const EMU_PT: f64 = 12_700.0;

/// 流程圖 and its 助手 through this app's routing (the shared engine settings).
struct FlowLlm(Shared);

#[async_trait::async_trait]
impl lore_flow::llm::Llm for FlowLlm {
    async fn complete_json(&self, task: &str, req: lore_llm::LlmRequest) -> Result<(Value, String), String> {
        let a = crate::todeck::llm(&self.0).complete_json(task, req, None).await.map_err(|(m, _)| m)?;
        Ok((a.value, a.engine))
    }
}

/// The registry in force for a deck: the global one and the deck's own.
fn deck_notation(s: &Shared, name: &str) -> Option<lore_notation::Notation> {
    let sess = session(s, name).ok()?;
    let dir = sess.lock().unwrap().dir.clone();
    let (glob, doc) = notation::load(&dir, global_notation(s).as_deref());
    Some(lore_notation::Notation::effective(&glob, &doc))
}

/// `/flow/*` (no app state needed).
pub(crate) fn flow_router(shared: &Shared) -> axum::Router {
    let s = shared.clone();
    let f = lore_flow::web::Flow::new(shared.home.clone()).with_app_dir(shared.ws.root.join("flow")).with_llm(Arc::new(FlowLlm(shared.clone()))).with_notation(Arc::new(move |doc: &str| deck_notation(&s, doc)));
    lore_flow::web::router(f)
}

pub(crate) fn routes() -> axum::Router<Shared> {
    axum::Router::new()
        .route("/api/decks/{name}/flow", axum::routing::post(insert).layer(axum::extract::DefaultBodyLimit::max(8 * 1024 * 1024)))
        .route("/api/decks/{name}/image", axum::routing::post(image))
}

#[derive(Deserialize)]
struct InsertBody {
    slide: String,
    spec: Value,
    #[serde(default)]
    replace: Option<u32>,
}

/// Where a new figure goes: its natural size, at most 90 % of the slide, centred.
fn placed(pic: &Object, slide_w: i64, slide_h: i64) -> (i64, i64, i64) {
    let (w, h) = (pic.frame.w as f64, pic.frame.h as f64);
    let k = (slide_w as f64 * 0.9 / w).min(slide_h as f64 * 0.8 / h).min(1.0);
    let (nw, nh) = (w * k, h * k);
    (((slide_w as f64 - nw) / 2.0).round() as i64, ((slide_h as f64 - nh) / 2.0).round() as i64, nw.round() as i64)
}

async fn insert(State(s): State<Shared>, Path(name): Path<String>, Json(b): Json<InsertBody>) -> R {
    let spec = FlowSpec::from_value(b.spec).map_err(bad)?;
    let sess = session(&s, &name)?;
    let (slide_w, slide_h, first_id, old, index) = {
        let sess = sess.lock().unwrap();
        let d = &sess.editor.deck;
        let sl = d.slide(&b.slide).ok_or_else(|| bad("找不到投影片"))?;
        let old = match b.replace {
            Some(id) => {
                let (parent, idx) = sl.locate(id).ok_or_else(|| bad("找不到那張圖"))?;
                if parent.is_some() {
                    return Err(bad("只能換掉最上層的圖"));
                }
                Some((sl.objects[idx].clone(), idx))
            }
            None => None,
        };
        let index = old.as_ref().map(|o| o.1).unwrap_or(sl.objects.len());
        (d.size.w, d.size.h, sl.max_id() + 1, old.map(|o| o.0), index)
    };
    let dirs: Vec<std::path::PathBuf> = s.home.shared_dir().map(|d| d.join("flow")).into_iter().chain([s.ws.root.join("flow")]).collect();
    let cat = Catalogue::load_all(&dirs.iter().map(|d| d.as_path()).collect::<Vec<_>>());
    let notation = deck_notation(&s, &name);
    let pandoc = s.pandoc.clone();
    let spec2 = spec.clone();
    // pandoc (Office Math) runs off the async threads
    let (pic, vars_new) = tokio::task::spawn_blocking(move || -> Result<(Object, Vec<lore_flow::VarNew>), String> {
        let math = lore_flow::PandocMath::new(pandoc);
        let frame = spec2.frame_rect();
        let (fig, g) = render(&spec2, &cat, notation.as_ref(), &math, Rect::new(0.0, 0.0, frame.w, frame.h), first_id)?;
        let id = fig.built.next_id;
        Ok((as_picture(&fig, &g.fig, &spec2, id), g.vars_new.clone()))
    })
    .await
    .map_err(|e| bad(e.to_string()))?
    .map_err(bad)?;
    // the figure's own variables join the deck's registry (never the global one)
    if !vars_new.is_empty() {
        let sess = sess.lock().unwrap();
        let path = sess.dir.join("notation.json");
        let mut doc = lore_notation::Notation::load(&path).unwrap_or_default();
        if !lore_flow::notation::register_new(&mut doc, &vars_new).is_empty() {
            doc.save(&path).map_err(bad)?;
        }
    }
    let (x, y, w) = match &old {
        // re-edit: the same place and width, the height follows the new figure
        Some(o) => (o.frame.x, o.frame.y, o.frame.w),
        None => placed(&pic, slide_w, slide_h),
    };
    let obj = lore_canvas::geom::scaled_to(&pic, x, y, w);
    let mut ops = Vec::new();
    if let Some(o) = &old {
        ops.push(Op::DeleteObj { slide: b.slide.clone(), id: o.id });
    }
    let id = obj.id;
    ops.push(Op::InsertObj { slide: b.slide.clone(), parent: None, index, obj });
    Ok(Json(json!({ "ops": ops, "id": id })))
}

#[derive(Deserialize)]
struct ImageBody {
    slide: String,
    id: u32,
    format: String,
}

fn find(objs: &[Object], id: u32) -> Option<&Object> {
    for o in objs {
        if o.id == id {
            return Some(o);
        }
        if let Kind::Group(g) = &o.kind
            && let Some(c) = find(&g.children, id)
        {
            return Some(c);
        }
    }
    None
}

fn urlenc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

/// A file name from an object's name (`flow:seg2` → `seg2`).
fn stem(deck: &str, o: &Object) -> String {
    let n = o.name.trim_start_matches("flow:");
    let n: String = n.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    let n = n.trim_matches('-');
    if n.is_empty() { format!("{deck}-{}", o.id) } else { format!("{deck}-{n}") }
}

/// One object or group as a picture: PNG (headless Chrome screenshot of `object.html`, 2×),
/// PDF (Chrome prints `object.html` at the object's size), SVG (from that PDF through
/// poppler's `pdftocairo`; without it a plain drawing of the objects, maths as Unicode).
/// **Not part of the offline tests** for PNG/PDF (needs a browser).
async fn image(State(s): State<Shared>, Path(name): Path<String>, Json(b): Json<ImageBody>) -> R {
    let fmt = b.format.to_lowercase();
    if !["png", "svg", "pdf"].contains(&fmt.as_str()) {
        return Err(bad("格式只能是 png、svg、pdf"));
    }
    let (obj, dir) = {
        let sess = session(&s, &name)?;
        let sess = sess.lock().unwrap();
        let sl = sess.editor.deck.slide(&b.slide).ok_or_else(|| bad("找不到投影片"))?;
        (find(&sl.objects, b.id).cloned().ok_or_else(|| bad("找不到物件"))?, sess.dir.join("exports"))
    };
    std::fs::create_dir_all(&dir).map_err(|e| bad(e.to_string()))?;
    let out = sc_core::free_path(&dir, &stem(&name, &obj), &fmt);
    let port = *s.port.lock().unwrap();
    let url = format!("http://127.0.0.1:{port}/object.html?deck={}&slide={}&obj={}", urlenc(&name), urlenc(&b.slide), b.id);
    let (w, h) = (((obj.frame.w as f64) / 9525.0).ceil().max(1.0) as u32, ((obj.frame.h as f64) / 9525.0).ceil().max(1.0) as u32);
    let out2 = out.clone();
    let res = tokio::task::spawn_blocking(move || -> Result<Option<String>, String> {
        let chrome = sc_core::find_chrome();
        let pdf = |path: &std::path::Path| -> Result<(), String> {
            let c = chrome.as_ref().ok_or("找不到 Chrome，無法輸出")?;
            sc_core::print_pdf(c, &format!("{url}&print=1"), path).map_err(|e| e.to_string())
        };
        match fmt.as_str() {
            "png" => {
                let c = chrome.as_ref().ok_or("找不到 Chrome，無法輸出 PNG")?;
                sc_core::screenshot_png(c, &url, w, h, &out2).map_err(|e| e.to_string())?;
                Ok(None)
            }
            "pdf" => {
                pdf(&out2)?;
                Ok(None)
            }
            _ => {
                // the PDF's drawing as SVG (same look, text as paths); else the plain drawing
                let tmp = out2.with_extension("tmp.pdf");
                let via_pdf = pdf(&tmp).ok().and_then(|_| std::fs::read(&tmp).ok()).and_then(|bytes| lore_plot::export::svg_from_pdf(&lore_plot::Tools::find(), &bytes));
                let _ = std::fs::remove_file(&tmp);
                match via_pdf {
                    Some(svg) => {
                        std::fs::write(&out2, svg).map_err(|e| e.to_string())?;
                        Ok(None)
                    }
                    None => {
                        let mut o = obj.clone();
                        o.frame.x = 0;
                        o.frame.y = 0;
                        let svg = lore_flow::preview::svg(&[o], obj.frame.w as f64 / EMU_PT, obj.frame.h as f64 / EMU_PT, 1.0);
                        std::fs::write(&out2, svg).map_err(|e| e.to_string())?;
                        Ok(Some("沒有 Chrome 或 pdftocairo：SVG 裡的數學式是一般文字".into()))
                    }
                }
            }
        }
    })
    .await
    .map_err(|e| bad(e.to_string()))?
    .map_err(bad)?;
    Ok(Json(json!({ "path": out, "warning": res })))
}
