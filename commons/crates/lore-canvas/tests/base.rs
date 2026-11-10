//! Canvas bases (底版) compile to a template.pptx that the usual layout path reads back.

use std::collections::BTreeMap;

use lore_canvas::base::{Base, compile};
use lore_canvas::{Kind, Paint, pptx_in};
use lore_ooxml::Deck as ODeck;

const BASE: &str = r#"{
  "id": "slant-test", "name": "斜切封面＋頁首內頁",
  "size": { "w": 12192000, "h": 6858000 },
  "fonts": { "major": "Calibri", "minor": "Calibri" },
  "pages": {
    "cover": {
      "layout": "封面",
      "title": { "x": 609600, "y": 1645920, "w": 6096000, "h": 914400 },
      "starter": [
        { "id": 2, "name": "slant", "frame": { "x": 0, "y": 0, "w": 7437120, "h": 6725107 }, "type": "shape",
          "geom": { "prst": "custom", "path": "M0 0 L1 0 L0.705 1 L0 1 Z" }, "fill": { "kind": "solid", "color": "@inverse" } }
      ]
    },
    "content": {
      "layout": "內頁",
      "bg": { "kind": "solid", "color": "@slide-bg" },
      "title": { "x": 487680, "y": 630936, "w": 11216640, "h": 534924 },
      "content": { "x": 487680, "y": 1447038, "w": 11216640, "h": 4039362 },
      "fixed": [
        { "id": 2, "name": "band", "frame": { "x": 0, "y": 6727698, "w": 12192000, "h": 130302 }, "type": "shape",
          "geom": { "prst": "rect" },
          "fill": { "kind": "gradient", "angle": 0, "stops": [ { "pos": 0, "color": "@band-1" }, { "pos": 0.4, "color": "@band-2" }, { "pos": 1, "color": "@band-3" } ] } },
        { "id": 3, "name": "page-number", "frame": { "x": 10997184, "y": 6487668, "w": 1194816, "h": 192024 }, "type": "text",
          "geom": { "prst": "rect" }, "fill": { "kind": "none" },
          "text": { "anchor": "b", "paras": [ { "align": "r", "runs": [ { "kind": "field", "field": "slidenum", "text": "‹#›", "look": { "font": "Calibri", "size": 10.0, "color": "@muted" } } ] } ] } }
      ]
    }
  }
}"#;

fn scheme() -> BTreeMap<String, String> {
    [("inverse", "1B2A1E"), ("slide-bg", "FFFFFF"), ("ink", "1B2A1E"), ("muted", "5B6270"), ("accent-1", "F2A516"), ("band-1", "00DEA4"), ("band-2", "33CCCC"), ("band-3", "35CBB2")]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn roles_resolve_and_typos_fail() {
    let b = Base::from_json(BASE, &scheme()).unwrap();
    let band = &b.pages["content"].fixed[0];
    let Kind::Shape(sp) = &band.kind else { panic!("band is a shape") };
    let Paint::Gradient { stops, .. } = &sp.fill else { panic!("band is a gradient") };
    assert_eq!(stops[1].color, "33CCCC");
    let typo = BASE.replace("@muted", "@mutd");
    assert!(Base::from_json(&typo, &scheme()).unwrap_err().contains("@mutd"));
    let no_content = BASE.replace("\"content\": {\n      \"layout\": \"內頁\"", "\"end\": {\n      \"layout\": \"內頁\"");
    assert!(Base::from_json(&no_content, &scheme()).is_err());
}

#[test]
fn compiles_to_a_template_the_layout_path_reads() {
    let b = Base::from_json(BASE, &scheme()).unwrap();
    let bytes = compile(&b, &scheme(), &|_: &str| None).unwrap();
    let od = ODeck::from_bytes(&bytes).unwrap();
    assert_eq!(od.slide_size, (12192000, 6858000));
    assert!(od.slides.is_empty());
    let names: Vec<&str> = od.layouts.iter().map(|l| l.name()).collect();
    assert_eq!(names, ["封面", "內頁"]);

    // title placeholders where the base says
    let content = od.layout_by_name("內頁").unwrap();
    let ph: Vec<String> = content.doc.placeholders().into_iter().map(|(p, _)| p.kind().to_string()).collect();
    assert_eq!(ph, ["title"]);
    let cover = od.layout_by_name("封面").unwrap();
    assert_eq!(cover.doc.placeholders().into_iter().map(|(p, _)| p.kind().to_string()).collect::<Vec<_>>(), ["ctrTitle"]);

    // fixed elements come back as backdrop objects: the gradient band and the page number field
    let bd = pptx_in::backdrops(&od);
    let inner = bd.iter().find(|b| b.layout == "內頁").unwrap();
    assert_eq!(inner.objects.len(), 2, "{:?}", inner.objects.iter().map(|o| &o.name).collect::<Vec<_>>());
    let Kind::Shape(sp) = &inner.objects[0].kind else { panic!("band") };
    let Paint::Gradient { stops, .. } = &sp.fill else { panic!("gradient survived: {:?}", sp.fill) };
    assert_eq!(stops.iter().map(|s| s.color.as_str()).collect::<Vec<_>>(), ["00DEA4", "33CCCC", "35CBB2"]);
    assert!((stops[1].pos - 0.4).abs() < 1e-6);
    let Kind::Text(tx) = &inner.objects[1].kind else { panic!("page number") };
    let run = &tx.text.as_ref().unwrap().paras[0].runs[0];
    assert_eq!((run.kind.as_str(), run.field.as_deref()), ("field", Some("slidenum")));
    assert_eq!(inner.bg, Paint::solid("FFFFFF"));

    // starter objects are not in the layout
    assert!(bd.iter().find(|b| b.layout == "封面").unwrap().objects.is_empty());

    // theme carries the scheme
    let theme = &od.themes[0];
    assert!(theme.colors.iter().any(|(k, v)| k == "dk1" && v == "1B2A1E"), "{:?}", theme.colors);
}

#[test]
fn polygon_vertices_are_connection_sites() {
    use lore_canvas::geom::{path_vertices, sites};
    assert_eq!(path_vertices("M0.5 0 L1 1 L0 1 Z"), [(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)]);
    // a closed path that repeats its start keeps it once; curves end on their last point
    assert_eq!(path_vertices("M0 0 L1 0 C1 0.5 0.5 1 0 1 L0 0 Z"), [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)]);
    let o: lore_canvas::Object = serde_json::from_value(serde_json::json!({
        "id": 5, "name": "tri", "frame": { "x": 0, "y": 0, "w": 100, "h": 200, "flip_h": true }, "type": "shape",
        "geom": { "prst": "custom", "path": "M0.25 0 L1 1 L0 1 Z" }, "fill": { "kind": "none" } })).unwrap();
    assert_eq!(sites(&o), [(75.0, 0.0), (0.0, 200.0), (100.0, 200.0)]);
    let xml = lore_canvas::pptx_out::cust_geom("M0.25 0 L1 1 L0 1 Z").to_xml_string();
    assert_eq!(xml.matches("<a:cxn ").count(), 3, "{xml}");
    assert!(xml.contains(r#"fmla="*/ w 25000 100000""#), "{xml}");
}
