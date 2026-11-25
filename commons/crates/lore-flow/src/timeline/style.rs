//! The style file (package T-style). Signatures are frozen; see `timeline/mod.rs`.

use std::path::Path;

use super::Style;

/// Parse one style file. `stem` = the file name without `.toml`, used as the id when the file
/// gives none. Unknown tables and keys are kept in [`Style::other`]; a value that is not TOML at
/// all, or a known field of the wrong type, is an `Err` of one line. [`check`] is run on the result.
pub fn parse(text: &str, stem: &str) -> Result<Style, String> {
    let mut style: Style = toml::from_str(text).map_err(|e| one_line(&e, text))?;
    if style.id.trim().is_empty() {
        style.id = stem.to_string();
    }
    if style.name.trim().is_empty() {
        style.name = style.id.clone();
    }
    check(&mut style);
    Ok(style)
}

/// A toml error as one short line: the message, and the line number when the position is known.
fn one_line(e: &toml::de::Error, text: &str) -> String {
    let msg = e.message().split_whitespace().collect::<Vec<_>>().join(" ");
    match e.span() {
        Some(span) => {
            let line = text.get(..span.start.min(text.len())).map_or(1, |t| t.matches('\n').count() + 1);
            format!("line {line}: {msg}")
        }
        None => msg,
    }
}

fn is_hex(s: &str) -> bool {
    s.len() == 6 && s.chars().all(|c| c.is_ascii_hexdigit())
}

const DASHES: [&str; 7] = ["solid", "dash", "sysDash", "sysDot", "lgDash", "dashDot", "lgDashDot"];
const FIRST: [&str; 2] = ["above", "below"];

/// Fill [`Style::cannot`]: every field whose value is not in [`super::can`], every colour that
/// is neither `point` / `paper` / `none` (where the field allows it) nor six hex digits, an
/// empty palette, a size that is not positive, and every table in [`Style::other`]. One short
/// line each, `table.key = "value"`. Returns the same list.
pub fn check(style: &mut Style) -> Vec<String> {
    use super::can;
    let mut out: Vec<String> = vec![];
    let mut one_of = |key: &str, value: &str, list: &[&str]| {
        if !list.contains(&value) {
            out.push(format!("{key} = \"{value}\""));
        }
    };
    one_of("axis.kind", &style.axis.kind, &can::AXIS);
    one_of("axis.at", &style.axis.at, &can::AXIS_AT);
    one_of("axis.dash", &style.axis.dash, &DASHES);
    one_of("marker.kind", &style.marker.kind, &can::MARKER);
    one_of("marker.inside", &style.marker.inside, &can::INSIDE);
    one_of("time.at", &style.time.at, &can::TIME_AT);
    one_of("text.kind", &style.text.kind, &can::TEXT);
    one_of("text.side", &style.text.side, &can::SIDE);
    one_of("text.first", &style.text.first, &FIRST);
    one_of("text.align", &style.text.align, &can::ALIGN);
    one_of("text.rule", &style.text.rule, &can::RULE);
    one_of("stem.kind", &style.stem.kind, &can::STEM);
    one_of("stem.end", &style.stem.end, &can::STEM_END);
    one_of("icon.at", &style.icon.at, &can::ICON_AT);
    one_of("colors.mode", &style.colors.mode, &can::COLORS);
    one_of("spacing", &style.spacing, &can::SPACING);
    one_of("title.align", &style.title.align, &can::ALIGN);

    // colours: the words a field allows besides six hex digits
    let mut colour = |key: &str, value: &str, words: &[&str]| {
        if !is_hex(value) && !words.contains(&value) {
            out.push(format!("{key} = \"{value}\""));
        }
    };
    colour("axis.color", &style.axis.color, &["point"]);
    colour("marker.fill", &style.marker.fill, &["point", "paper"]);
    colour("marker.line", &style.marker.line, &["point", "paper", "none"]);
    colour("time.color", &style.time.color, &["point"]);
    colour("text.title_color", &style.text.title_color, &["point"]);
    colour("text.body_color", &style.text.body_color, &["point"]);
    colour("stem.color", &style.stem.color, &["point"]);
    colour("card.fill", &style.card.fill, &["point"]);
    colour("card.text", &style.card.text, &[]);
    colour("colors.ink", &style.colors.ink, &[]);
    colour("colors.muted", &style.colors.muted, &[]);
    colour("title.color", &style.title.color, &[]);
    for c in &style.colors.palette {
        if !is_hex(c) {
            out.push(format!("colors.palette = \"{c}\""));
        }
    }

    // sizes (NaN is reported too)
    let mut positive = |key: &str, v: f64| {
        if v.is_nan() || v <= 0.0 {
            out.push(format!("{key} = {v}"));
        }
    };
    positive("axis.weight", style.axis.weight);
    positive("marker.size", style.marker.size);
    positive("time.size", style.time.size);
    positive("text.title_size", style.text.title_size);
    positive("text.body_size", style.text.body_size);
    positive("icon.size", style.icon.size);
    positive("title.size", style.title.size);
    let mut not_negative = |key: &str, v: f64| {
        if v.is_nan() || v < 0.0 {
            out.push(format!("{key} = {v}"));
        }
    };
    not_negative("stem.weight", style.stem.weight);
    not_negative("stem.length", style.stem.length);
    not_negative("marker.line_weight", style.marker.line_weight);
    not_negative("marker.radius", style.marker.radius);
    not_negative("card.radius", style.card.radius);
    not_negative("card.pad", style.card.pad);
    not_negative("text.width", style.text.width);

    for key in style.other.keys() {
        out.push(format!("{key}: not drawn yet"));
    }
    style.cannot = out;
    style.cannot.clone()
}

/// The neutral styles ([`super::BUILTIN`]), parsed; one that does not parse is a bug and panics
/// in tests, and is skipped at run time.
pub fn builtin() -> Vec<Style> {
    super::BUILTIN.iter().filter_map(|(id, text)| parse(text, id).ok()).collect()
}

/// `<dir>/timelines/*.toml`, sorted by file name, each marked [`Style::private`]. A file that
/// does not parse is skipped and its name and error returned in the second list.
pub fn load_dir(dir: &Path) -> (Vec<Style>, Vec<String>) {
    let Ok(read) = std::fs::read_dir(dir.join("timelines")) else { return (vec![], vec![]) };
    let mut files: Vec<std::path::PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().and_then(|x| x.to_str()) == Some("toml"))
        .collect();
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    let (mut styles, mut broken) = (vec![], vec![]);
    for p in files {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        let stem = p.file_stem().and_then(|n| n.to_str()).unwrap_or_default();
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| parse(&t, stem)) {
            Ok(mut s) => {
                s.private = true;
                styles.push(s);
            }
            Err(e) => broken.push(format!("{name}: {e}")),
        }
    }
    (styles, broken)
}

/// The style a spec asks for: by id, else the first of `styles`, else [`Style::default`].
pub fn pick<'a>(styles: &'a [Style], id: &str) -> std::borrow::Cow<'a, Style> {
    match styles.iter().find(|s| s.id == id).or(styles.first()) {
        Some(s) => std::borrow::Cow::Borrowed(s),
        None => std::borrow::Cow::Owned(Style::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_states_only_what_differs() {
        let s = parse("[marker]\nkind = \"ring\"\n", "x").unwrap();
        assert_eq!(s.marker.kind, "ring");
        assert_eq!(s.marker.size, Style::default().marker.size);
        assert_eq!(s.axis, Style::default().axis);
        assert_eq!(s.colors, Style::default().colors);
    }

    #[test]
    fn the_stem_names_a_style_without_an_id() {
        let s = parse("", "my-style").unwrap();
        assert_eq!(s.id, "my-style");
        assert_eq!(s.name, "my-style");
        let s = parse("id = \"a\"\nname = \"N\"", "b").unwrap();
        assert_eq!((s.id.as_str(), s.name.as_str()), ("a", "N"));
    }

    #[test]
    fn a_wrong_type_is_one_line() {
        let e = parse("[axis]\nweight = \"thick\"\n", "x").unwrap_err();
        assert!(!e.is_empty() && !e.contains('\n'), "{e:?}");
    }

    #[test]
    fn text_that_is_not_toml_is_one_line() {
        let e = parse("[axis\nweight = = 3\n", "x").unwrap_err();
        assert!(!e.is_empty() && !e.contains('\n'), "{e:?}");
    }

    #[test]
    fn the_default_style_draws_as_written() {
        assert!(check(&mut Style::default()).is_empty());
    }

    #[test]
    fn unknown_values_are_listed_not_fixed() {
        let s = parse("[axis]\nkind = \"path\"\ndash = \"wavy\"\n[text]\nfirst = \"left\"\n", "x").unwrap();
        assert_eq!(s.axis.kind, "path");
        assert_eq!(s.cannot, vec!["axis.kind = \"path\"", "axis.dash = \"wavy\"", "text.first = \"left\""]);
        let s = parse("spacing = \"date\"", "x").unwrap();
        assert_eq!(s.cannot, vec!["spacing = \"date\""]);
    }

    #[test]
    fn colours_must_be_hex_or_a_keyword() {
        let s = parse("[axis]\ncolor = \"paper\"\n[marker]\nfill = \"paper\"\nline = \"none\"\n[card]\ntext = \"point\"\n[colors]\nink = \"aBc123\"\n[title]\ncolor = \"abc12G\"\n", "x").unwrap();
        assert_eq!(s.cannot, vec!["axis.color = \"paper\"", "card.text = \"point\"", "title.color = \"abc12G\""]);
        let s = parse("[colors]\npalette = [\"4C6EF5\", \"#12B886\"]\n", "x").unwrap();
        assert_eq!(s.cannot, vec!["colors.palette = \"#12B886\""]);
    }

    #[test]
    fn an_empty_palette_means_lores_own_colours() {
        let s = parse("[colors]\npalette = []\n", "x").unwrap();
        assert!(s.cannot.is_empty());
        assert!(Style::default().colors.palette.is_empty(), "a style states a palette only when it must differ");
    }

    #[test]
    fn sizes_must_be_positive() {
        let s = parse("[axis]\nweight = 0\n[marker]\nsize = -2\nradius = 0\n[stem]\nweight = 0\nlength = -1\n[text]\nwidth = 0\n", "x").unwrap();
        assert_eq!(s.cannot, vec!["axis.weight = 0", "marker.size = -2", "stem.length = -1"]);
    }

    #[test]
    fn unknown_tables_are_reported() {
        let s = parse("[[track]]\nname = \"a\"\n[today]\nat = 3\n", "x").unwrap();
        assert_eq!(s.cannot, vec!["today: not drawn yet", "track: not drawn yet"]);
    }

    #[test]
    fn checking_twice_gives_the_same_list() {
        let mut s = parse("[axis]\nkind = \"path\"\n[today]\n", "x").unwrap();
        let a = check(&mut s);
        let b = check(&mut s);
        assert_eq!(a, b);
        assert_eq!(s.cannot, a);
        assert_eq!(a.len(), 2);
    }

    #[test]
    fn the_neutral_styles_parse_and_draw_as_written() {
        let styles = builtin();
        let ids: Vec<&str> = styles.iter().map(|s| s.id.as_str()).collect();
        let want: Vec<&str> = super::super::BUILTIN.iter().map(|(i, _)| *i).collect();
        assert_eq!(ids, want);
        assert_eq!(styles.len(), 3);
        for s in &styles {
            assert!(s.cannot.is_empty(), "{}: {:?}", s.id, s.cannot);
            assert!(!s.private);
            assert!(!s.name.is_empty());
        }
    }

    fn temp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("lore-flow-style-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("timelines")).unwrap();
        d
    }

    #[test]
    fn a_folder_of_styles_is_read_in_name_order_and_marked_private() {
        let d = temp("order");
        let t = d.join("timelines");
        std::fs::write(t.join("b.toml"), "name = \"B\"").unwrap();
        std::fs::write(t.join("a.toml"), "id = \"first\"").unwrap();
        std::fs::write(t.join("c.txt"), "x").unwrap();
        std::fs::create_dir_all(t.join("sub.toml")).unwrap();
        let (styles, broken) = load_dir(&d);
        let _ = std::fs::remove_dir_all(&d);
        assert!(broken.is_empty());
        let ids: Vec<&str> = styles.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["first", "b"]);
        assert!(styles.iter().all(|s| s.private));
    }

    #[test]
    fn a_broken_file_is_named_not_dropped() {
        let d = temp("broken");
        std::fs::write(d.join("timelines").join("bad.toml"), "[axis\n").unwrap();
        std::fs::write(d.join("timelines").join("ok.toml"), "").unwrap();
        let (styles, broken) = load_dir(&d);
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(styles.len(), 1);
        assert_eq!(broken.len(), 1);
        assert!(broken[0].starts_with("bad.toml: ") && !broken[0].contains('\n'), "{broken:?}");
    }

    #[test]
    fn a_missing_folder_is_no_styles() {
        let d = std::env::temp_dir().join(format!("lore-flow-style-missing-{}", std::process::id()));
        assert_eq!(load_dir(&d), (vec![], vec![]));
    }

    #[test]
    fn a_style_that_cannot_be_drawn_is_still_loaded() {
        let d = temp("cannot");
        std::fs::write(d.join("timelines").join("odd.toml"), "[axis]\nkind = \"path\"\n").unwrap();
        let (styles, broken) = load_dir(&d);
        let _ = std::fs::remove_dir_all(&d);
        assert!(broken.is_empty());
        assert_eq!(styles.len(), 1);
        assert!(!styles[0].cannot.is_empty());
    }

    #[test]
    fn a_style_is_picked_by_id_else_the_first_else_the_default() {
        let list = vec![parse("", "a").unwrap(), parse("", "b").unwrap()];
        assert_eq!(pick(&list, "b").id, "b");
        assert_eq!(pick(&list, "zzz").id, "a");
        assert_eq!(*pick(&[], "b"), Style::default());
    }
}
