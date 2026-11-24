//! Row-shaped formats (K2): pattern, expression, rule, exemplar, wording, and the generic CSV
//! [`CsvAdapter`] that presents an app's existing CSV file as items and writes changes back into
//! the same file (docs/brain-hub.md §4, §5, §12).
//!
//! Row numbers in problems count like a spreadsheet: the header is row 1, the first data row is 2.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

use crate::types::*;
use crate::util::write_atomic;

// ── the five formats ─────────────────────────────────────────────────────────

fn row_def(id: &str, label: &str, fields: Vec<FieldDef>) -> FormatDef {
    FormatDef { id: id.into(), label: label.into(), fields, shape: "row".into(), form: true }
}

fn choice(name: &str, label: &str, choices: &[&str]) -> FieldDef {
    let mut f = FieldDef::new(name, label, "choice", false);
    f.choices = choices.iter().map(|s| s.to_string()).collect();
    f
}

/// Non-empty parts joined.
fn join(parts: &[&str], sep: &str) -> String {
    parts.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(sep)
}

/// `[a/b] ` or "" when every part is empty.
fn bracket(parts: &[&str]) -> String {
    let j = join(parts, "/");
    if j.is_empty() { String::new() } else { format!("[{j}] ") }
}

/// A field, or the item's title when the field is the title column and is empty.
fn field_or_title<'a>(item: &'a Item, name: &str) -> &'a str {
    let v = item.field(name).trim();
    if v.is_empty() { item.title.as_str() } else { v }
}

fn with_example(mut s: String, item: &Item) -> String {
    let e = item.field("example").trim();
    if !e.is_empty() {
        s.push_str("\ne.g. ");
        s.push_str(e);
    }
    s
}

/// 句型.
pub struct Pattern;
/// 常用詞.
pub struct Expression;
/// 規則.
pub struct Rule;
/// 範例.
pub struct Exemplar;
/// 用詞對照.
pub struct Wording;

impl Format for Pattern {
    fn def(&self) -> FormatDef {
        row_def(format::PATTERN, "句型", vec![
            FieldDef::new("section", "段落", "text", false),
            FieldDef::new("function", "功能", "text", false),
            choice("position", "位置", &["opening", "body", "closing", "transition"]),
            FieldDef::new("template", "句型", "text", true),
            FieldDef::new("example", "例句", "text", false),
            FieldDef::new("priority", "優先", "number", false),
        ])
    }
    fn line(&self, item: &Item) -> String {
        join(&[item.field("section"), item.field("function"), item.field("position")], " · ")
    }
    fn prompt(&self, item: &Item) -> String {
        let s = format!("{}{}", bracket(&[item.field("section"), item.field("function"), item.field("position")]), field_or_title(item, "template"));
        with_example(s, item)
    }
}

impl Format for Expression {
    fn def(&self) -> FormatDef {
        row_def(format::EXPRESSION, "常用詞", vec![
            choice("kind", "種類", &["collocation", "phrase", "connective"]),
            FieldDef::new("text", "詞語", "text", true),
            FieldDef::new("usage", "用法", "text", false),
            FieldDef::new("section", "段落", "text", false),
            FieldDef::new("example", "例句", "text", false),
        ])
    }
    fn line(&self, item: &Item) -> String {
        join(&[item.field("kind"), item.field("usage")], " · ")
    }
    fn prompt(&self, item: &Item) -> String {
        let mut s = format!("{}{}", bracket(&[item.field("kind")]), field_or_title(item, "text"));
        let u = item.field("usage").trim();
        if !u.is_empty() {
            s.push_str(&format!(" ({u})"));
        }
        with_example(s, item)
    }
}

impl Format for Rule {
    fn def(&self) -> FormatDef {
        row_def(format::RULE, "規則", vec![
            choice("type", "類型", &["hard", "soft"]),
            FieldDef::new("scope", "範圍", "text", false),
            FieldDef::new("check", "檢查", "text", false),
            FieldDef::new("threshold", "門檻", "number", false),
            FieldDef::new("severity", "嚴重度", "text", false),
            FieldDef::new("message", "說明", "text", true),
        ])
    }
    fn line(&self, item: &Item) -> String {
        join(&[item.field("type"), item.field("scope"), item.field("severity")], " · ")
    }
    fn prompt(&self, item: &Item) -> String {
        let m = field_or_title(item, "message");
        let p = join(&[item.field("scope"), item.field("severity")], ", ");
        if p.is_empty() { m.to_string() } else { format!("{m} ({p})") }
    }
}

impl Format for Exemplar {
    fn def(&self) -> FormatDef {
        row_def(format::EXEMPLAR, "範例", vec![
            FieldDef::new("section", "段落", "text", false),
            FieldDef::new("function", "功能", "text", false),
            FieldDef::new("text", "內文", "text", true),
            FieldDef::new("source", "出處", "text", false),
        ])
    }
    fn line(&self, item: &Item) -> String {
        join(&[item.field("section"), item.field("function")], " · ")
    }
    fn prompt(&self, item: &Item) -> String {
        format!("{}{}", bracket(&[item.field("section"), item.field("function")]), item.field("text").trim())
    }
}

impl Format for Wording {
    fn def(&self) -> FormatDef {
        row_def(format::WORDING, "用詞對照", vec![
            FieldDef::new("avoid", "避免", "text", true),
            FieldDef::new("prefer", "改用", "text", false),
            FieldDef::new("note", "備註", "text", false),
        ])
    }
    fn line(&self, item: &Item) -> String {
        format!("→ {}", item.field("prefer").trim())
    }
    fn prompt(&self, item: &Item) -> String {
        let mut s = format!("avoid \"{}\"; write \"{}\"", field_or_title(item, "avoid"), item.field("prefer").trim());
        let n = item.field("note").trim();
        if !n.is_empty() {
            s.push_str(&format!(" ({n})"));
        }
        s
    }
}

/// The five row formats.
pub fn all() -> Vec<Box<dyn Format>> {
    vec![Box::new(Pattern), Box::new(Expression), Box::new(Rule), Box::new(Exemplar), Box::new(Wording)]
}

/// The items of one category as a prompt block: one `prompt()` per item, in the order given.
pub fn render(format: &dyn Format, items: &[&Item]) -> String {
    items.iter().map(|i| format.prompt(i)).collect::<Vec<_>>().join("\n")
}

// ── the CSV adapter ──────────────────────────────────────────────────────────

/// What a CSV format looks like on disk.
struct Spec {
    header: &'static [&'static str],
    /// Has an `id` column (wording has none: its local id is `safe_local(avoid)`).
    id_col: bool,
    tags_col: bool,
    /// The column the title comes from (also the fallback for a new item without that field).
    title_col: &'static str,
    /// Exemplar's title is derived from `text`, never written back.
    title_writes_back: bool,
}

fn spec(fmt: &str) -> Result<&'static Spec> {
    static PATTERN: Spec = Spec { header: &["id", "section", "function", "position", "template", "example", "tags", "priority"], id_col: true, tags_col: true, title_col: "template", title_writes_back: true };
    static EXPRESSION: Spec = Spec { header: &["id", "kind", "text", "usage", "section", "example", "tags"], id_col: true, tags_col: true, title_col: "text", title_writes_back: true };
    static RULE: Spec = Spec { header: &["id", "type", "scope", "check", "threshold", "severity", "message"], id_col: true, tags_col: false, title_col: "message", title_writes_back: true };
    static EXEMPLAR: Spec = Spec { header: &["id", "section", "function", "text", "source"], id_col: true, tags_col: false, title_col: "text", title_writes_back: false };
    static WORDING: Spec = Spec { header: &["avoid", "prefer", "note"], id_col: false, tags_col: false, title_col: "avoid", title_writes_back: true };
    match fmt {
        format::PATTERN => Ok(&PATTERN),
        format::EXPRESSION => Ok(&EXPRESSION),
        format::RULE => Ok(&RULE),
        format::EXEMPLAR => Ok(&EXEMPLAR),
        format::WORDING => Ok(&WORDING),
        other => Err(KnowError::Invalid(format!("不是表格格式：{other}"))),
    }
}

/// An app's CSV file of one row format, as the items of one category.
///
/// Wording rows have no id column: the local id is `safe_local(avoid)`. Changing `avoid` through
/// [`Adapter::save`] changes the row but the item keeps its id (the row is found by the id it had).
#[derive(Debug, Clone)]
pub struct CsvAdapter {
    path: PathBuf,
    category: String,
    format: String,
    source: Source,
}

struct Table {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl CsvAdapter {
    pub fn new(path: impl Into<PathBuf>, category: &str, format: &str, source: Source) -> CsvAdapter {
        CsvAdapter { path: path.into(), category: category.into(), format: format.into(), source }
    }

    /// `None` when the file does not exist.
    fn read(&self, sp: &Spec) -> Result<Option<Table>> {
        let bytes = match fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let mut rd = csv::ReaderBuilder::new().has_headers(false).flexible(true).from_reader(bytes.as_slice());
        let mut recs: Vec<Vec<String>> = vec![];
        for r in rd.records() {
            let r = r.map_err(|e| KnowError::Invalid(format!("讀不了 {}：{e}", self.path.display())))?;
            recs.push(r.iter().map(str::to_string).collect());
        }
        if recs.is_empty() {
            return Ok(Some(Table { header: sp.header.iter().map(|s| s.to_string()).collect(), rows: vec![] }));
        }
        let header = recs.remove(0);
        Ok(Some(Table { header, rows: recs }))
    }

    /// Atomic (the shared helper).
    fn write(&self, t: &Table) -> Result<()> {
        let mut w = csv::WriterBuilder::new().flexible(true).from_writer(vec![]);
        w.write_record(&t.header).map_err(|e| KnowError::Other(e.to_string()))?;
        for r in &t.rows {
            w.write_record(r).map_err(|e| KnowError::Other(e.to_string()))?;
        }
        let bytes = w.into_inner().map_err(|e| KnowError::Other(e.to_string()))?;
        write_atomic(&self.path, &bytes)
    }

    fn item_of(&self, sp: &Spec, header: &[String], row: &[String], local: &str) -> Item {
        let mut it = Item::new(&self.category, &self.format, "", self.source.clone());
        it.id = item_id(&self.category, local);
        for (j, name) in header.iter().enumerate() {
            if name == "id" || name == "tags" {
                continue;
            }
            let c = row.get(j).map(String::as_str).unwrap_or("");
            it.fields.insert(name.clone(), Value::String(c.to_string()));
        }
        if sp.tags_col {
            for t in cell(header, row, "tags").split(';') {
                it.tag(facet::KIND, t);
            }
        }
        it.title = match self.format.as_str() {
            format::EXEMPLAR => cell(header, row, "text").split_whitespace().collect::<Vec<_>>().join(" ").chars().take(60).collect(),
            format::RULE => {
                let m = cell(header, row, "message").trim();
                if m.is_empty() { local.to_string() } else { m.to_string() }
            }
            _ => cell(header, row, sp.title_col).trim().to_string(),
        };
        it.tidy();
        it
    }
}

fn cell<'a>(header: &[String], row: &'a [String], name: &str) -> &'a str {
    header.iter().position(|h| h == name).and_then(|j| row.get(j)).map(String::as_str).unwrap_or("")
}

/// The row's local id, "" when it has none.
fn local_of(sp: &Spec, header: &[String], row: &[String]) -> String {
    if sp.id_col {
        cell(header, row, "id").trim().to_string()
    } else {
        let a = cell(header, row, "avoid").trim();
        if a.is_empty() { String::new() } else { safe_local(a) }
    }
}

fn as_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(as_text).collect::<Vec<_>>().join(";"),
        other => other.to_string(),
    }
}

fn split_tags(s: &str) -> Vec<String> {
    let mut v: Vec<String> = s.split(';').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    v.sort();
    v.dedup();
    v
}

impl Adapter for CsvAdapter {
    fn load(&self) -> Result<(Vec<Item>, Vec<Problem>)> {
        let sp = spec(&self.format)?;
        let Some(t) = self.read(sp)? else { return Ok((vec![], vec![])) };
        let (mut items, mut problems) = (vec![], vec![]);
        let mut seen = std::collections::HashSet::new();
        for (i, row) in t.rows.iter().enumerate() {
            let at = format!("#{}", i + 2);
            let bad = |m: String| Problem { at: at.clone(), field: if sp.id_col { "id".into() } else { "avoid".into() }, message: m };
            let local = local_of(sp, &t.header, row);
            if local.is_empty() {
                problems.push(bad(format!("第 {} 列沒有{}，略過", i + 2, if sp.id_col { " id" } else { "「避免」" })));
            } else if local.contains(':') || local.contains('/') {
                problems.push(bad(format!("第 {} 列的 id 含 : 或 /，略過", i + 2)));
            } else if !seen.insert(local.clone()) {
                problems.push(bad(format!("第 {} 列的 id「{local}」重複，略過", i + 2)));
            } else {
                items.push(self.item_of(sp, &t.header, row, &local));
            }
        }
        Ok((items, problems))
    }

    fn save(&self, item: &Item) -> Result<()> {
        let sp = spec(&self.format)?;
        let mut t = self.read(sp)?.unwrap_or_else(|| Table { header: sp.header.iter().map(|s| s.to_string()).collect(), rows: vec![] });
        if t.header.is_empty() {
            t.header = sp.header.iter().map(|s| s.to_string()).collect();
        }
        for k in item.fields.keys() {
            if k == "id" || k == "tags" || !t.header.contains(k) {
                return Err(KnowError::Invalid(format!("這個格式沒有「{k}」欄")));
            }
        }
        let local = split_id(&item.id).map(|(_, l)| l.to_string()).ok_or_else(|| KnowError::Invalid(format!("id 不對：{}", item.id)))?;
        let at = t.rows.iter().position(|r| local_of(sp, &t.header, r) == local);
        let is_new = at.is_none();
        let mut row = match at {
            Some(i) => t.rows[i].clone(),
            None => vec![],
        };
        let header = t.header.clone();
        // Only a cell whose value differs is touched, so unchanged rows stay byte for byte.
        let set = |row: &mut Vec<String>, j: usize, v: String| {
            if row.get(j).map(String::as_str).unwrap_or("") != v {
                if row.len() <= j {
                    row.resize(j + 1, String::new());
                }
                row[j] = v;
            }
        };
        for (j, name) in header.iter().enumerate() {
            if name == "id" {
                if is_new {
                    set(&mut row, j, local.clone());
                }
            } else if name == "tags" {
                let want = split_tags(&item.tags.get(facet::KIND).map(|v| v.join(";")).unwrap_or_default());
                if is_new || split_tags(row.get(j).map(String::as_str).unwrap_or("")) != want {
                    set(&mut row, j, want.join(";"));
                }
            } else if let Some(v) = item.fields.get(name) {
                set(&mut row, j, as_text(v));
            } else if is_new && sp.title_writes_back && name == sp.title_col {
                set(&mut row, j, item.title.clone());
            }
        }
        // A row without an id column is known by its `avoid` text: it must be there, and be new.
        if !sp.id_col {
            let now_local = local_of(sp, &header, &row);
            if now_local.is_empty() {
                return Err(KnowError::Invalid("「避免」不能空白".into()));
            }
            if let Some(other) = t.rows.iter().enumerate().find(|(i, r)| Some(*i) != at && local_of(sp, &header, r) == now_local) {
                return Err(KnowError::Invalid(format!("已經有一列「避免」是「{}」了", cell(&header, other.1, "avoid").trim())));
            }
        }
        match at {
            Some(i) => t.rows[i] = row,
            None => {
                row.resize(row.len().max(header.len()), String::new());
                t.rows.push(row);
            }
        }
        self.write(&t)
    }

    fn remove(&self, id: &str) -> Result<Item> {
        let sp = spec(&self.format)?;
        let local = split_id(id).map(|(_, l)| l.to_string()).ok_or_else(|| KnowError::NotFound(id.into()))?;
        let mut t = self.read(sp)?.ok_or_else(|| KnowError::NotFound(id.into()))?;
        let at = t.rows.iter().position(|r| local_of(sp, &t.header, r) == local).ok_or_else(|| KnowError::NotFound(id.into()))?;
        let row = t.rows.remove(at);
        let item = self.item_of(sp, &t.header, &row, &local);
        self.write(&t)?;
        Ok(item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn it(fmt: &str, fields: &[(&str, &str)]) -> Item {
        let mut i = Item::new("t.cat", fmt, "", Source::author());
        for (k, v) in fields {
            i.set(k, *v);
        }
        i
    }

    #[test]
    fn each_format_declares_its_columns() {
        let want: [(&str, &[&str], &str); 5] = [
            (format::PATTERN, &["section", "function", "position", "template", "example", "priority"], "句型"),
            (format::EXPRESSION, &["kind", "text", "usage", "section", "example"], "常用詞"),
            (format::RULE, &["type", "scope", "check", "threshold", "severity", "message"], "規則"),
            (format::EXEMPLAR, &["section", "function", "text", "source"], "範例"),
            (format::WORDING, &["avoid", "prefer", "note"], "用詞對照"),
        ];
        let all = all();
        assert_eq!(all.len(), 5);
        for (f, (id, cols, label)) in all.iter().zip(want) {
            let d = f.def();
            assert_eq!((d.id.as_str(), d.label.as_str(), d.shape.as_str(), d.form), (id, label, "row", true));
            let names: Vec<&str> = d.fields.iter().map(|x| x.name.as_str()).collect();
            assert_eq!(names, cols);
            // the declared columns are the adapter's columns (minus id and tags)
            let sp = spec(id).unwrap();
            let mut a = names.clone();
            let mut b: Vec<&str> = sp.header.iter().copied().filter(|c| *c != "id" && *c != "tags").collect();
            a.sort();
            b.sort();
            assert_eq!(a, b);
            assert_eq!(d.fields.iter().filter(|x| x.required).map(|x| x.name.as_str()).collect::<Vec<_>>(), vec![sp.title_col]);
            assert!(d.fields.iter().all(|x| matches!(x.kind.as_str(), "text" | "choice" | "number")));
            assert!(d.fields.iter().all(|x| x.kind != "choice" || !x.choices.is_empty()));
        }
    }

    #[test]
    fn required_fields_are_checked() {
        for (f, title_col) in [(format::PATTERN, "template"), (format::EXPRESSION, "text"), (format::RULE, "message"), (format::EXEMPLAR, "text"), (format::WORDING, "avoid")] {
            let fm = all().into_iter().find(|x| x.def().id == f).unwrap();
            let mut i = it(f, &[]);
            i.title = "x".into();
            let p = fm.validate(&i);
            assert_eq!(p.len(), 1, "{f}");
            assert_eq!(p[0].field, title_col);
            i.set(title_col, "something");
            assert!(fm.validate(&i).is_empty(), "{f}");
        }
    }

    #[test]
    fn lines_and_prompts_read_well() {
        let p = it(format::PATTERN, &[("section", "intro"), ("function", "gap"), ("position", "opening"), ("template", "Little is known about X."), ("example", "Little is known about tea.")]);
        assert_eq!(Pattern.line(&p), "intro · gap · opening");
        assert_eq!(Pattern.prompt(&p), "[intro/gap/opening] Little is known about X.\ne.g. Little is known about tea.");
        let p2 = it(format::PATTERN, &[("template", "We propose X.")]);
        assert_eq!(Pattern.prompt(&p2), "We propose X.");

        let e = it(format::EXPRESSION, &[("kind", "connective"), ("text", "in contrast"), ("usage", "contrast two results")]);
        assert_eq!(Expression.line(&e), "connective · contrast two results");
        assert_eq!(Expression.prompt(&e), "[connective] in contrast (contrast two results)");

        let r = it(format::RULE, &[("type", "soft"), ("scope", "abstract"), ("severity", "warn"), ("message", "Keep it under 200 words.")]);
        assert_eq!(Rule.line(&r), "soft · abstract · warn");
        assert_eq!(Rule.prompt(&r), "Keep it under 200 words. (abstract, warn)");

        let x = it(format::EXEMPLAR, &[("section", "intro"), ("function", "hook"), ("text", "Tea is old.")]);
        assert_eq!(Exemplar.line(&x), "intro · hook");
        assert_eq!(Exemplar.prompt(&x), "[intro/hook] Tea is old.");

        let w = it(format::WORDING, &[("avoid", "utilize"), ("prefer", "use"), ("note", "plain")]);
        assert_eq!(Wording.line(&w), "→ use");
        assert_eq!(Wording.prompt(&w), "avoid \"utilize\"; write \"use\" (plain)");
    }

    const PAT: &str = "id,section,function,position,template,example,tags,priority\n\
P1,intro,gap,opening,Little is known about X.,Little is known about tea.,b;a,2\n\
P2,method,step,body,We then do X.,,,1\n";

    fn adapter(dir: &Path, name: &str, fmt: &str) -> CsvAdapter {
        CsvAdapter::new(dir.join(name), "t.cat", fmt, Source::lore("pc"))
    }

    #[test]
    fn rows_become_items_with_tags() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("p.csv"), PAT).unwrap();
        let (items, problems) = adapter(d.path(), "p.csv", format::PATTERN).load().unwrap();
        assert!(problems.is_empty());
        assert_eq!(items.len(), 2);
        let a = &items[0];
        assert_eq!((a.id.as_str(), a.title.as_str(), a.field("position"), a.field("priority")), ("t.cat:P1", "Little is known about X.", "opening", "2"));
        assert_eq!(a.tags[facet::KIND], vec!["a", "b"]);
        assert!(a.entity && a.source == Source::lore("pc") && !a.fields.contains_key("id") && !a.fields.contains_key("tags"));
        assert!(items[1].tags.is_empty());
        // wording: no id column
        fs::write(d.path().join("w.csv"), "avoid,prefer,note\nutilize,use,\n").unwrap();
        let (w, _) = adapter(d.path(), "w.csv", format::WORDING).load().unwrap();
        assert_eq!((w[0].id.as_str(), w[0].title.as_str()), ("t.cat:utilize", "utilize"));
        // exemplar title: one line, 60 chars
        let long = "word ".repeat(30);
        fs::write(d.path().join("x.csv"), format!("id,section,function,text,source\nE1,a,b,\"line one\nline two {long}\",s\n")).unwrap();
        let (x, _) = adapter(d.path(), "x.csv", format::EXEMPLAR).load().unwrap();
        assert!(x[0].title.starts_with("line one line two word") && x[0].title.chars().count() <= 60);
        // rule title falls back to id
        fs::write(d.path().join("r.csv"), "id,type,scope,check,threshold,severity,message\nR1,hard,all,len,3,error,\n").unwrap();
        let (r, _) = adapter(d.path(), "r.csv", format::RULE).load().unwrap();
        assert_eq!(r[0].title, "R1");
    }

    #[test]
    fn bad_rows_are_listed_not_fatal() {
        let d = tempfile::tempdir().unwrap();
        let s = "id,kind,text,usage,section,example,tags\nA,phrase,one,,,,\n,phrase,no id,,,,\nA,phrase,again,,,,\nB:C,phrase,colon,,,,\nD,phrase,fine,,,,\n";
        fs::write(d.path().join("e.csv"), s).unwrap();
        let (items, problems) = adapter(d.path(), "e.csv", format::EXPRESSION).load().unwrap();
        assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["t.cat:A", "t.cat:D"]);
        assert_eq!(problems.iter().map(|p| p.at.as_str()).collect::<Vec<_>>(), ["#3", "#4", "#5"]);
        assert!(problems[0].message.contains('3') && problems[1].message.contains("重複"));
    }

    #[test]
    fn a_missing_file_is_empty() {
        let d = tempfile::tempdir().unwrap();
        let (i, p) = adapter(d.path(), "none.csv", format::PATTERN).load().unwrap();
        assert!(i.is_empty() && p.is_empty());
    }

    const ODD: &str = "id,section,function,position,template,example,tags,priority,extra\n\
P1,intro,gap,opening,\"Hello, \"\"X\"\"\nsecond line\",,b;a,2,keep me\n\
P2,method,step,body,We then do X.,,,1,\n";

    #[test]
    fn load_then_save_unchanged_leaves_the_file_identical() {
        let d = tempfile::tempdir().unwrap();
        let a = adapter(d.path(), "p.csv", format::PATTERN);
        fs::write(&a.path, ODD).unwrap();
        let (items, _) = a.load().unwrap();
        assert_eq!(items[0].field("extra"), "keep me");
        for i in &items {
            a.save(i).unwrap();
        }
        assert_eq!(fs::read_to_string(&a.path).unwrap(), ODD);
    }

    #[test]
    fn a_changed_item_changes_only_its_row() {
        let d = tempfile::tempdir().unwrap();
        let a = adapter(d.path(), "p.csv", format::PATTERN);
        fs::write(&a.path, ODD).unwrap();
        let (mut items, _) = a.load().unwrap();
        items[1].set("template", "We next do X.");
        items[1].tag(facet::KIND, "method");
        a.save(&items[1]).unwrap();
        let s = fs::read_to_string(&a.path).unwrap();
        let first_two = ODD.split("P2,").next().unwrap();
        assert_eq!(s, format!("{first_two}P2,method,step,body,We next do X.,,method,1,\n"));
        // wording: renaming avoid keeps the id
        let w = adapter(d.path(), "w.csv", format::WORDING);
        fs::write(&w.path, "avoid,prefer,note\nutilize,use,\n").unwrap();
        let (mut wi, _) = w.load().unwrap();
        wi[0].set("avoid", "utilise");
        w.save(&wi[0]).unwrap();
        assert_eq!(fs::read_to_string(&w.path).unwrap(), "avoid,prefer,note\nutilise,use,\n");
        assert_eq!(wi[0].id, "t.cat:utilize");
    }

    #[test]
    fn a_new_item_is_appended_and_reloads_equal() {
        let d = tempfile::tempdir().unwrap();
        let a = adapter(d.path(), "new/p.csv", format::PATTERN);
        let mut n = it(format::PATTERN, &[("section", "intro"), ("template", "New X."), ("position", "body")]);
        n.id = item_id("t.cat", "P9");
        n.title = "New X.".into();
        n.tag(facet::KIND, "z").tag(facet::KIND, "y");
        n.source = Source::lore("pc");
        a.save(&n).unwrap();
        let s = fs::read_to_string(&a.path).unwrap();
        assert!(s.starts_with("id,section,function,position,template,example,tags,priority\nP9,intro,,body,New X.,,y;z,"), "{s}");
        let (items, p) = a.load().unwrap();
        assert!(p.is_empty());
        assert_eq!(items.len(), 1);
        assert_eq!((items[0].id.as_str(), &items[0].title, &items[0].tags), (n.id.as_str(), &n.title, &n.tags));
        assert_eq!(items[0].field("template"), "New X.");
        // appended after existing rows
        let mut m = n.clone();
        m.id = item_id("t.cat", "P10");
        a.save(&m).unwrap();
        assert_eq!(a.load().unwrap().0.len(), 2);
    }

    #[test]
    fn remove_takes_the_row_out_and_returns_it() {
        let d = tempfile::tempdir().unwrap();
        let a = adapter(d.path(), "p.csv", format::PATTERN);
        fs::write(&a.path, ODD).unwrap();
        let (items, _) = a.load().unwrap();
        let gone = a.remove("t.cat:P1").unwrap();
        assert_eq!(gone, items[0]);
        let (rest, _) = a.load().unwrap();
        assert_eq!(rest, vec![items[1].clone()]);
        assert!(matches!(a.remove("t.cat:P1"), Err(KnowError::NotFound(_))));
    }

    #[test]
    fn an_unknown_field_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let a = adapter(d.path(), "p.csv", format::PATTERN);
        fs::write(&a.path, PAT).unwrap();
        let (mut items, _) = a.load().unwrap();
        items[0].set("colour", "red");
        match a.save(&items[0]) {
            Err(KnowError::Invalid(m)) => assert!(m.contains("colour")),
            other => panic!("{other:?}"),
        }
        assert_eq!(fs::read_to_string(&a.path).unwrap(), PAT);
    }

    #[test]
    fn a_block_is_one_item_per_entry() {
        let a = it(format::WORDING, &[("avoid", "utilize"), ("prefer", "use")]);
        let b = it(format::WORDING, &[("avoid", "in order to"), ("prefer", "to"), ("note", "short")]);
        assert_eq!(render(&Wording, &[&a, &b]), "avoid \"utilize\"; write \"use\"\navoid \"in order to\"; write \"to\" (short)");
        assert_eq!(render(&Wording, &[]), "");
    }
}
