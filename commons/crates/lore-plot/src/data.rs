//! Data in: CSV / TSV / pasted cells, JSON, XLSX (every sheet) and LaTeX `tabular`s
//! (through lore-docsrc). Cells become numbers when they read as numbers; `0.81 ± 0.02`
//! columns are split into a value column and a `<name>_sd` column.

use std::io::Read;

use serde_json::Value;

use crate::spec::Table;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("讀不了這個檔案：{0}")]
    Read(String),
    #[error("檔案裡沒有表格")]
    Empty,
}

/// Tables in a file, by its name's extension (`.csv .tsv .txt .json .xlsx .tex`).
pub fn import(name: &str, bytes: &[u8]) -> Result<Vec<Table>, ImportError> {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let stem = name.to_string();
    let tables = match ext.as_str() {
        "xlsx" => xlsx(bytes)?,
        _ => {
            let text = String::from_utf8_lossy(bytes);
            let text = text.trim_start_matches('\u{feff}');
            if ext == "tex" || text.contains("\\begin{tabular") {
                latex(text)
            } else if ext == "json" || text.trim_start().starts_with(['[', '{']) {
                json(text).into_iter().collect()
            } else {
                delimited(text).into_iter().collect()
            }
        }
    };
    let tables: Vec<Table> = tables
        .into_iter()
        .filter(|t| !t.columns.is_empty() && !t.rows.is_empty())
        .enumerate()
        .map(|(i, mut t)| {
            if t.name.is_empty() {
                t.name = if i == 0 { stem.clone() } else { format!("{stem} ({})", i + 1) };
            }
            split_pm(t)
        })
        .collect();
    if tables.is_empty() { Err(ImportError::Empty) } else { Ok(tables) }
}

/// A cell as JSON: a number when it reads as one (`1,234`, `−0.5`, `12%` stays text).
pub fn cell(s: &str) -> Value {
    let t = s.trim();
    if t.is_empty() {
        return Value::Null;
    }
    let n = t.replace(['−', '–'], "-").replace(',', "");
    if n.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '+' || c == '.')
        && let Ok(v) = n.parse::<f64>()
        && v.is_finite()
    {
        return serde_json::Number::from_f64(v).map(Value::Number).unwrap_or(Value::Null);
    }
    Value::String(t.to_string())
}

/// CSV, TSV or `;`-separated text (pasted spreadsheet cells are tab-separated); first row = names.
pub fn delimited(text: &str) -> Option<Table> {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let first = *lines.first()?;
    let sep = if first.contains('\t') {
        '\t'
    } else if first.contains(';') && !first.contains(',') {
        ';'
    } else {
        ','
    };
    let split = |l: &str| -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = String::new();
        let mut q = false;
        let mut it = l.chars().peekable();
        while let Some(c) = it.next() {
            if q {
                if c == '"' && it.peek() == Some(&'"') {
                    cur.push('"');
                    it.next();
                } else if c == '"' {
                    q = false;
                } else {
                    cur.push(c);
                }
            } else if c == '"' {
                q = true;
            } else if c == sep {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.push(c);
            }
        }
        out.push(cur);
        out.into_iter().map(|s| s.trim().to_string()).collect()
    };
    let head = split(first);
    let columns: Vec<String> = head.iter().enumerate().map(|(i, c)| if c.is_empty() { format!("col{}", i + 1) } else { c.clone() }).collect();
    let rows = lines[1..]
        .iter()
        .map(|l| {
            let cells = split(l);
            (0..columns.len()).map(|i| cells.get(i).map(|c| cell(c)).unwrap_or(Value::Null)).collect()
        })
        .collect();
    Some(Table { name: String::new(), columns: unique(columns), rows })
}

/// `[{…}, …]` or `{ "columns": [...], "rows": [[...]] }`.
pub fn json(text: &str) -> Option<Table> {
    let v: Value = serde_json::from_str(text).ok()?;
    if let (Some(c), Some(r)) = (v.get("columns").and_then(Value::as_array), v.get("rows").and_then(Value::as_array)) {
        let columns: Vec<String> = c.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect();
        let rows = r.iter().filter_map(Value::as_array).map(|row| (0..columns.len()).map(|i| norm(row.get(i))).collect()).collect();
        return Some(Table { name: String::new(), columns, rows });
    }
    let arr = v.as_array()?;
    let mut columns: Vec<String> = Vec::new();
    for o in arr.iter().filter_map(Value::as_object) {
        for k in o.keys() {
            if !columns.contains(k) {
                columns.push(k.clone());
            }
        }
    }
    let rows = arr.iter().filter_map(Value::as_object).map(|o| columns.iter().map(|c| norm(o.get(c))).collect()).collect();
    Some(Table { name: String::new(), columns, rows })
}

fn norm(v: Option<&Value>) -> Value {
    match v {
        Some(Value::String(s)) => cell(s),
        Some(v @ (Value::Number(_) | Value::Null)) => v.clone(),
        Some(Value::Bool(b)) => Value::from(u8::from(*b)),
        Some(other) => Value::String(other.to_string()),
        None => Value::Null,
    }
}

fn unique(cols: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in cols {
        let mut name = c.clone();
        let mut k = 2;
        while out.contains(&name) {
            name = format!("{c}_{k}");
            k += 1;
        }
        out.push(name);
    }
    out
}

/// Every `tabular`-like environment in LaTeX source, cells turned into plain text.
pub fn latex(text: &str) -> Vec<Table> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = text[at..].find("\\begin{tabular") {
        let start = at + i;
        let Some(grid) = lore_docsrc::latex::tabular_grid(&text[start..]) else { break };
        let rows: Vec<Vec<String>> = grid.into_iter().map(|r| r.iter().map(|c| tex_cell(c)).collect()).filter(|r: &Vec<String>| r.iter().any(|c| !c.is_empty())).collect();
        if let Some((head, body)) = rows.split_first() {
            let width = rows.iter().map(Vec::len).max().unwrap_or(0);
            let columns: Vec<String> = (0..width).map(|k| head.get(k).filter(|c| !c.is_empty()).cloned().unwrap_or_else(|| format!("col{}", k + 1))).collect();
            let rows = body.iter().map(|r| (0..width).map(|k| r.get(k).map(|c| cell(c)).unwrap_or(Value::Null)).collect()).collect();
            out.push(Table { name: String::new(), columns: unique(columns), rows });
        }
        at = start + "\\begin{tabular".len();
    }
    out
}

/// LaTeX cell → plain text: formatting commands dropped, `\pm` kept as ±, maths unwrapped.
fn tex_cell(s: &str) -> String {
    let mut t = s.replace("\\pm", "±").replace("\\%", "%").replace("\\,", "").replace('~', " ").replace("\\&", "&");
    for cmd in ["\\textbf", "\\textit", "\\emph", "\\mathbf", "\\mathrm", "\\underline", "\\textrm", "\\textsc", "\\boldsymbol", "\\text"] {
        t = t.replace(cmd, "");
    }
    // \cellcolor{…}, \color{…}: drop with their argument
    for cmd in ["\\cellcolor", "\\color", "\\rowcolor"] {
        while let Some(i) = t.find(cmd) {
            let rest = &t[i + cmd.len()..];
            let end = if rest.starts_with('[') { rest.find(']').map(|j| j + 1).unwrap_or(0) } else { 0 };
            let rest2 = &rest[end..];
            let skip = if rest2.starts_with('{') { rest2.find('}').map(|j| j + 1).unwrap_or(rest2.len()) } else { 0 };
            t = format!("{}{}", &t[..i], &rest2[skip..]);
        }
    }
    t.chars().filter(|c| !matches!(c, '{' | '}' | '$')).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Columns whose cells read `a ± b` → `col` (a) and `col_sd` (b).
pub fn split_pm(t: Table) -> Table {
    let pm: Vec<bool> = (0..t.columns.len()).map(|i| t.rows.iter().any(|r| r.get(i).and_then(Value::as_str).is_some_and(|s| s.contains('±')))).collect();
    if !pm.iter().any(|b| *b) {
        return t;
    }
    let mut columns = Vec::new();
    for (i, c) in t.columns.iter().enumerate() {
        columns.push(c.clone());
        if pm[i] {
            columns.push(format!("{c}_sd"));
        }
    }
    let rows = t
        .rows
        .iter()
        .map(|r| {
            let mut out = Vec::new();
            for (i, v) in r.iter().enumerate() {
                if !pm.get(i).copied().unwrap_or(false) {
                    out.push(v.clone());
                    continue;
                }
                match v.as_str().and_then(|s| s.split_once('±')) {
                    Some((a, b)) => {
                        out.push(cell(a));
                        out.push(cell(b));
                    }
                    None => {
                        out.push(v.clone());
                        out.push(Value::Null);
                    }
                }
            }
            out
        })
        .collect();
    Table { name: t.name, columns: unique(columns), rows }
}

// ── XLSX: every sheet, shared strings, numbers; no formulas evaluated (cached values) ──

/// Sheets of an .xlsx workbook as tables (first row = names).
pub fn xlsx(bytes: &[u8]) -> Result<Vec<Table>, ImportError> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| ImportError::Read(e.to_string()))?;
    let mut read = |name: &str| -> Option<String> {
        let mut f = zip.by_name(name).ok()?;
        let mut s = String::new();
        f.read_to_string(&mut s).ok()?;
        Some(s)
    };
    let shared = read("xl/sharedStrings.xml").map(|x| shared_strings(&x)).unwrap_or_default();
    let workbook = read("xl/workbook.xml").ok_or_else(|| ImportError::Read("不是 Excel 活頁簿（沒有 workbook.xml）".into()))?;
    let rels = read("xl/_rels/workbook.xml.rels").unwrap_or_default();
    let mut out = Vec::new();
    for (name, rid) in sheets(&workbook) {
        let target = rel_target(&rels, &rid).unwrap_or_else(|| format!("worksheets/sheet{}.xml", out.len() + 1));
        let path = if let Some(t) = target.strip_prefix('/') { t.to_string() } else { format!("xl/{target}") };
        let Some(xml) = read(&path) else { continue };
        let grid = sheet_cells(&xml, &shared);
        let Some((head, body)) = grid.split_first() else { continue };
        let width = grid.iter().map(Vec::len).max().unwrap_or(0);
        let columns: Vec<String> = (0..width).map(|k| head.get(k).and_then(|v| v.as_ref()).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("col{}", k + 1))).collect();
        let rows = body.iter().filter(|r| r.iter().any(Option::is_some)).map(|r| (0..width).map(|k| r.get(k).cloned().flatten().unwrap_or(Value::Null)).collect()).collect();
        out.push(Table { name, columns: unique(columns), rows });
    }
    Ok(out)
}

fn attr(e: &quick_xml::events::BytesStart, key: &str) -> Option<String> {
    e.attributes().flatten().find(|a| a.key.as_ref() == key || a.key.local_name().as_ref() == key).and_then(|a| a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok()).map(|v| v.into_owned())
}

fn shared_strings(xml: &str) -> Vec<String> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_si = false;
    let mut in_t = false;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) if e.local_name().as_ref() == "si" => {
                in_si = true;
                cur.clear();
            }
            Ok(Event::Start(e)) if e.local_name().as_ref() == "t" => in_t = in_si,
            Ok(Event::End(e)) if e.local_name().as_ref() == "t" => in_t = false,
            Ok(Event::End(e)) if e.local_name().as_ref() == "si" => {
                in_si = false;
                out.push(std::mem::take(&mut cur));
            }
            Ok(Event::Text(t)) if in_t => cur.push_str(&t.xml10_content()),
            Ok(Event::GeneralRef(g)) if in_t => cur.push_str(match g.xml10_content().into_owned().as_str() {
                "amp" => "&",
                "lt" => "<",
                "gt" => ">",
                "quot" => "\"",
                "apos" => "'",
                _ => "",
            }),
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn sheets(xml: &str) -> Vec<(String, String)> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut out = Vec::new();
    loop {
        match r.read_event() {
            Ok(Event::Empty(e) | Event::Start(e)) if e.local_name().as_ref() == "sheet" => {
                out.push((attr(&e, "name").unwrap_or_default(), attr(&e, "r:id").or_else(|| attr(&e, "id")).unwrap_or_default()));
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

fn rel_target(xml: &str, rid: &str) -> Option<String> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    loop {
        match r.read_event() {
            Ok(Event::Empty(e) | Event::Start(e)) if e.local_name().as_ref() == "Relationship" => {
                if attr(&e, "Id").as_deref() == Some(rid) {
                    return attr(&e, "Target");
                }
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
}

/// `A1` → column index 0.
fn col_index(r: &str) -> usize {
    r.chars().take_while(char::is_ascii_alphabetic).fold(0usize, |a, c| a * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1)).saturating_sub(1)
}

fn sheet_cells(xml: &str, shared: &[String]) -> Vec<Vec<Option<Value>>> {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let mut rows: Vec<Vec<Option<Value>>> = Vec::new();
    let mut col = 0usize;
    let mut kind = String::new();
    let mut text = String::new();
    let mut in_v = false;
    loop {
        match r.read_event() {
            Ok(Event::Start(e)) if e.local_name().as_ref() == "row" => rows.push(Vec::new()),
            Ok(Event::Start(e)) if e.local_name().as_ref() == "c" => {
                col = attr(&e, "r").map(|r| col_index(&r)).unwrap_or_else(|| rows.last().map(Vec::len).unwrap_or(0));
                kind = attr(&e, "t").unwrap_or_default();
                text.clear();
            }
            Ok(Event::Start(e)) if matches!(e.local_name().as_ref(), "v" | "t") => in_v = true,
            Ok(Event::End(e)) if matches!(e.local_name().as_ref(), "v" | "t") => in_v = false,
            Ok(Event::Text(t)) if in_v => text.push_str(&t.xml10_content()),
            Ok(Event::End(e)) if e.local_name().as_ref() == "c" => {
                let v = match kind.as_str() {
                    "s" => text.trim().parse::<usize>().ok().and_then(|i| shared.get(i)).map(|s| cell(s)),
                    "str" | "inlineStr" => Some(Value::String(text.clone())),
                    "b" => Some(Value::from(u8::from(text.trim() == "1"))),
                    _ if text.trim().is_empty() => None,
                    _ => Some(cell(&text)),
                };
                if let Some(row) = rows.last_mut() {
                    if row.len() <= col {
                        row.resize(col + 1, None);
                    }
                    row[col] = v;
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    rows
}
