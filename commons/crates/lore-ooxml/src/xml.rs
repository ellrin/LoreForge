//! Minimal owned XML DOM on top of quick-xml.
//!
//! Prefixed names are kept verbatim (`p:sp`, `a:rPr`), namespace declarations are
//! ordinary attributes. This keeps the round trip honest: whatever prefixes and
//! declarations the source used are written back as they were.

use std::fmt::Write as _;

use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Ordered attribute list (order is preserved on write; compare is order-insensitive).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attrs(pub Vec<(String, String)>);

impl Attrs {
    pub fn new() -> Self {
        Self(Vec::new())
    }
    pub fn get(&self, k: &str) -> Option<&str> {
        self.0.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }
    pub fn get_i64(&self, k: &str) -> Option<i64> {
        self.get(k).and_then(|v| v.parse().ok())
    }
    pub fn get_bool(&self, k: &str) -> Option<bool> {
        self.get(k).map(|v| v == "1" || v == "true")
    }
    /// Set (replace in place, or append).
    pub fn set(&mut self, k: &str, v: impl Into<String>) {
        let v = v.into();
        if let Some(slot) = self.0.iter_mut().find(|(n, _)| n == k) {
            slot.1 = v;
        } else {
            self.0.push((k.to_string(), v));
        }
    }
    pub fn set_opt(&mut self, k: &str, v: Option<impl Into<String>>) {
        match v {
            Some(v) => self.set(k, v),
            None => self.remove(k),
        }
    }
    pub fn remove(&mut self, k: &str) {
        self.0.retain(|(n, _)| n != k);
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0.iter().map(|(a, b)| (a.as_str(), b.as_str()))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Element(Element),
    Text(String),
    CData(String),
    Comment(String),
    PI(String),
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Element {
    pub name: String,
    pub attrs: Attrs,
    pub children: Vec<Node>,
}

impl Element {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            attrs: Attrs::new(),
            children: Vec::new(),
        }
    }
    pub fn with_attr(mut self, k: &str, v: impl Into<String>) -> Self {
        self.attrs.set(k, v);
        self
    }
    pub fn with_child(mut self, c: Element) -> Self {
        self.children.push(Node::Element(c));
        self
    }
    pub fn with_text(mut self, t: impl Into<String>) -> Self {
        self.children.push(Node::Text(t.into()));
        self
    }
    pub fn push(&mut self, c: Element) {
        self.children.push(Node::Element(c));
    }
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.get(k)
    }
    /// Local part of the name (`sp` for `p:sp`).
    pub fn local(&self) -> &str {
        local_name(&self.name)
    }
    pub fn is(&self, name: &str) -> bool {
        self.name == name
    }
    pub fn elements(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }
    pub fn elements_mut(&mut self) -> impl Iterator<Item = &mut Element> {
        self.children.iter_mut().filter_map(|n| match n {
            Node::Element(e) => Some(e),
            _ => None,
        })
    }
    pub fn child(&self, name: &str) -> Option<&Element> {
        self.elements().find(|e| e.name == name)
    }
    pub fn child_mut(&mut self, name: &str) -> Option<&mut Element> {
        self.elements_mut().find(|e| e.name == name)
    }
    /// Concatenated text of direct text children.
    pub fn text(&self) -> String {
        let mut s = String::new();
        for n in &self.children {
            match n {
                Node::Text(t) | Node::CData(t) => s.push_str(t),
                _ => {}
            }
        }
        s
    }
    /// Depth-first search for descendants (including self) with this name.
    pub fn descendants<'a>(&'a self, name: &'a str, out: &mut Vec<&'a Element>) {
        if self.name == name {
            out.push(self);
        }
        for c in self.elements() {
            c.descendants(name, out);
        }
    }
    pub fn find_all<'a>(&'a self, name: &'a str) -> Vec<&'a Element> {
        let mut v = Vec::new();
        self.descendants(name, &mut v);
        v
    }
    pub fn contains(&self, name: &str) -> bool {
        self.name == name || self.elements().any(|c| c.contains(name))
    }

    pub fn to_xml_string(&self) -> String {
        let mut s = String::new();
        write_element(self, &mut s);
        s
    }
}

pub fn local_name(n: &str) -> &str {
    n.rsplit_once(':').map(|(_, l)| l).unwrap_or(n)
}

/// A parsed XML part: declaration (verbatim) + whatever came between it and the root + root.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// Raw declaration text inside `<?` `?>`, e.g. `xml version="1.0" encoding="UTF-8" standalone="yes"`.
    pub decl: Option<String>,
    /// Whitespace / misc between declaration and root (PowerPoint writes "\r\n").
    pub prolog_ws: String,
    pub root: Element,
}

impl Document {
    pub fn new(root: Element) -> Self {
        Self {
            decl: Some(r#"xml version="1.0" encoding="UTF-8" standalone="yes""#.into()),
            prolog_ws: "\r\n".into(),
            root,
        }
    }
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        parse_document(bytes)
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut s = String::with_capacity(4096);
        if let Some(d) = &self.decl {
            s.push_str("<?");
            s.push_str(d);
            s.push_str("?>");
        }
        s.push_str(&self.prolog_ws);
        write_element(&self.root, &mut s);
        s.into_bytes()
    }
}

fn unescape(raw: &str) -> String {
    if !raw.contains('&') {
        return raw.to_string();
    }
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(j) = after.find(';') {
            let ent = &after[..j];
            if let Some(c) = resolve_entity(ent) {
                out.push(c);
                rest = &after[j + 1..];
                continue;
            }
        }
        out.push('&');
        rest = after;
    }
    out.push_str(rest);
    out
}

fn resolve_entity(ent: &str) -> Option<char> {
    match ent {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let n = if let Some(h) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                u32::from_str_radix(h, 16).ok()?
            } else {
                let d = ent.strip_prefix('#')?;
                d.parse().ok()?
            };
            char::from_u32(n)
        }
    }
}

fn parse_document(bytes: &[u8]) -> Result<Document> {
    let text = std::str::from_utf8(bytes).map_err(|e| Error::Xml(format!("not utf-8: {e}")))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut reader = Reader::from_str(text);
    {
        let c = reader.config_mut();
        c.trim_text(false);
        c.expand_empty_elements = false;
        c.check_end_names = true;
    }
    let mut decl = None;
    let mut prolog_ws = String::new();
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;

    fn push_text(stack: &mut [Element], t: String) {
        if let Some(top) = stack.last_mut() {
            if let Some(Node::Text(prev)) = top.children.last_mut() {
                prev.push_str(&t);
            } else {
                top.children.push(Node::Text(t));
            }
        }
    }

    loop {
        let ev = reader
            .read_event()
            .map_err(|e| Error::Xml(format!("{e} at {}", reader.buffer_position())))?;
        match ev {
            Event::Decl(d) => {
                let raw: &str = &d;
                decl = Some(raw.trim().to_string());
            }
            Event::Start(s) => {
                stack.push(start_to_element(&s)?);
            }
            Event::Empty(s) => {
                let el = start_to_element(&s)?;
                match stack.last_mut() {
                    Some(top) => top.children.push(Node::Element(el)),
                    None => root = Some(el),
                }
            }
            Event::End(_) => {
                let el = stack
                    .pop()
                    .ok_or_else(|| Error::Xml("unbalanced end tag".into()))?;
                match stack.last_mut() {
                    Some(top) => top.children.push(Node::Element(el)),
                    None => root = Some(el),
                }
            }
            Event::Text(t) => {
                if stack.is_empty() {
                    if root.is_none() {
                        // keep prolog whitespace raw ("\r\n" after the declaration)
                        let raw: &str = &t;
                        prolog_ws.push_str(raw);
                    }
                } else {
                    let s = t.xml10_content().into_owned();
                    push_text(&mut stack, s);
                }
            }
            Event::GeneralRef(r) => {
                let name = r.xml10_content().into_owned();
                let c = resolve_entity(&name)
                    .ok_or_else(|| Error::Xml(format!("unknown entity &{name};")))?;
                push_text(&mut stack, c.to_string());
            }
            Event::CData(c) => {
                let s = c.into_inner().into_owned();
                if let Some(top) = stack.last_mut() {
                    top.children.push(Node::CData(s));
                }
            }
            Event::Comment(c) => {
                let s = c.xml10_content().into_owned();
                if let Some(top) = stack.last_mut() {
                    top.children.push(Node::Comment(s));
                }
            }
            Event::PI(p) => {
                let raw: &str = &p;
                let s = raw.to_string();
                if let Some(top) = stack.last_mut() {
                    top.children.push(Node::PI(s));
                }
            }
            Event::DocType(_) => {}
            Event::Eof => break,
        }
    }
    let root = root.ok_or_else(|| Error::Xml("no root element".into()))?;
    Ok(Document {
        decl,
        prolog_ws,
        root,
    })
}

fn start_to_element(s: &quick_xml::events::BytesStart) -> Result<Element> {
    let name = s.name().as_ref().to_string();
    let mut attrs = Attrs::new();
    for a in s.attributes().with_checks(false) {
        let a = a.map_err(|e| Error::Xml(format!("attribute: {e}")))?;
        let k = a.key.as_ref().to_string();
        attrs.0.push((k, unescape(&a.value)));
    }
    Ok(Element {
        name,
        attrs,
        children: Vec::new(),
    })
}

pub fn parse_element(bytes: &[u8]) -> Result<Element> {
    Ok(parse_document(bytes)?.root)
}

fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(c),
        }
    }
}

fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            '\t' => out.push_str("&#x9;"),
            _ => out.push(c),
        }
    }
}

pub fn write_element(el: &Element, out: &mut String) {
    out.push('<');
    out.push_str(&el.name);
    for (k, v) in &el.attrs.0 {
        out.push(' ');
        out.push_str(k);
        out.push_str("=\"");
        escape_attr(v, out);
        out.push('"');
    }
    if el.children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for c in &el.children {
        match c {
            Node::Element(e) => write_element(e, out),
            Node::Text(t) => escape_text(t, out),
            Node::CData(t) => {
                out.push_str("<![CDATA[");
                out.push_str(t);
                out.push_str("]]>");
            }
            Node::Comment(t) => {
                out.push_str("<!--");
                out.push_str(t);
                out.push_str("-->");
            }
            Node::PI(t) => {
                out.push_str("<?");
                out.push_str(t);
                out.push_str("?>");
            }
        }
    }
    out.push_str("</");
    out.push_str(&el.name);
    out.push('>');
}

// ---------------------------------------------------------------------------
// Canonical comparison
// ---------------------------------------------------------------------------

/// Canonical text form: attributes sorted, comments dropped, whitespace-only text
/// dropped in element-only content (mixed text such as `<a:t> </a:t>` is kept),
/// empty elements and `<x></x>` treated alike.
pub fn canonical(el: &Element) -> String {
    let mut s = String::new();
    canon_into(el, &mut s, 0);
    s
}

fn canon_into(el: &Element, out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push(' ');
    }
    out.push('<');
    out.push_str(&el.name);
    let mut attrs: Vec<_> = el.attrs.0.iter().collect();
    attrs.sort();
    for (k, v) in attrs {
        let _ = write!(out, " {k}={v:?}");
    }
    out.push_str(">\n");
    let has_el = el.children.iter().any(|c| matches!(c, Node::Element(_)));
    for c in &el.children {
        match c {
            Node::Element(e) => canon_into(e, out, depth + 1),
            Node::Text(t) | Node::CData(t) => {
                if has_el && t.trim().is_empty() {
                    continue;
                }
                for _ in 0..=depth {
                    out.push(' ');
                }
                let _ = writeln!(out, "#text {t:?}");
            }
            Node::Comment(_) => {}
            Node::PI(p) => {
                let _ = writeln!(out, "#pi {p:?}");
            }
        }
    }
}

/// Compare two elements canonically. `Ok(())` when equal, otherwise the first
/// differing line pair (with line number) for diagnostics.
pub fn canonical_diff(a: &Element, b: &Element) -> std::result::Result<(), String> {
    let ca = canonical(a);
    let cb = canonical(b);
    if ca == cb {
        return Ok(());
    }
    for (i, (la, lb)) in ca.lines().zip(cb.lines()).enumerate() {
        if la != lb {
            return Err(format!(
                "line {}: `{}` vs `{}`",
                i + 1,
                la.trim(),
                lb.trim()
            ));
        }
    }
    Err(format!(
        "length differs: {} vs {} lines",
        ca.lines().count(),
        cb.lines().count()
    ))
}

pub fn canonical_eq_bytes(a: &[u8], b: &[u8]) -> Result<std::result::Result<(), String>> {
    let da = Document::parse(a)?;
    let db = Document::parse(b)?;
    Ok(canonical_diff(&da.root, &db.root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_write_roundtrip_preserves_text_and_entities() {
        let src = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<a:p xmlns:a=\"urn:x\"><a:t> a &amp; b &lt;c&gt; &#x4E2D; </a:t><a:e v=\"x&quot;y&#xA;\"/></a:p>";
        let d = Document::parse(src.as_bytes()).unwrap();
        let t = d.root.child("a:t").unwrap().text();
        assert_eq!(t, " a & b <c> 中 ");
        assert_eq!(d.root.child("a:e").unwrap().attr("v"), Some("x\"y\n"));
        let out = d.to_bytes();
        let d2 = Document::parse(&out).unwrap();
        assert_eq!(d, d2);
        assert!(
            String::from_utf8(out).unwrap().starts_with(
                "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n<a:p"
            )
        );
    }

    #[test]
    fn canonical_ignores_attr_order_and_formatting_whitespace() {
        let a = parse_element(b"<r><x b=\"2\" a=\"1\"/>\n  <t> </t></r>").unwrap();
        let b = parse_element(b"<r><x a=\"1\" b=\"2\"></x><t> </t></r>").unwrap();
        assert!(canonical_diff(&a, &b).is_ok());
        let c = parse_element(b"<r><x a=\"1\" b=\"2\"/><t></t></r>").unwrap();
        assert!(canonical_diff(&a, &c).is_err());
    }
}
