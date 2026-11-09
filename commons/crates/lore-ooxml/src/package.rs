//! OPC package: every zip entry kept as raw bytes, in original order, plus
//! helpers for relationships and content types.

use std::io::{Cursor, Read, Write};
use std::path::Path;

use crate::xml::{Document, Element};
use crate::{Error, Result};

pub const REL_SLIDE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slide";
pub const REL_SLIDE_LAYOUT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideLayout";
pub const REL_SLIDE_MASTER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/slideMaster";
pub const REL_NOTES_SLIDE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesSlide";
pub const REL_NOTES_MASTER: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/notesMaster";
pub const REL_THEME: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/theme";
pub const REL_IMAGE: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/image";
pub const REL_THUMBNAIL: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/metadata/thumbnail";
pub const CT_SLIDE: &str = "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";

#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub name: String,
    pub data: Vec<u8>,
    /// true when the source entry was stored (not deflated); kept on write.
    pub stored: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Package {
    pub parts: Vec<Part>,
}

impl Package {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let bytes = std::fs::read(path.as_ref())?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let mut z =
            zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Zip(e.to_string()))?;
        let mut parts = Vec::with_capacity(z.len());
        for i in 0..z.len() {
            let mut f = z.by_index(i).map_err(|e| Error::Zip(e.to_string()))?;
            if f.is_dir() {
                continue;
            }
            let mut data = Vec::with_capacity(f.size() as usize);
            f.read_to_end(&mut data)?;
            parts.push(Part {
                name: f.name().to_string(),
                stored: f.compression() == zip::CompressionMethod::Stored,
                data,
            });
        }
        Ok(Self { parts })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for p in &self.parts {
            let method = if p.stored {
                zip::CompressionMethod::Stored
            } else {
                zip::CompressionMethod::Deflated
            };
            let opts = zip::write::SimpleFileOptions::default()
                .compression_method(method)
                .large_file(p.data.len() as u64 >= u32::MAX as u64);
            w.start_file(p.name.as_str(), opts)
                .map_err(|e| Error::Zip(e.to_string()))?;
            w.write_all(&p.data)?;
        }
        let c = w.finish().map_err(|e| Error::Zip(e.to_string()))?;
        Ok(c.into_inner())
    }

    /// Write to `path`. Refuses to overwrite an existing file (house rule).
    pub fn save_new(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if path.exists() {
            return Err(Error::Exists(path.display().to_string()));
        }
        std::fs::write(path, self.to_bytes()?)?;
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.parts
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.data.as_slice())
    }
    pub fn has(&self, name: &str) -> bool {
        self.parts.iter().any(|p| p.name == name)
    }
    pub fn set(&mut self, name: &str, data: Vec<u8>) {
        if let Some(p) = self.parts.iter_mut().find(|p| p.name == name) {
            p.data = data;
        } else {
            self.parts.push(Part {
                name: name.to_string(),
                data,
                stored: false,
            });
        }
    }
    pub fn remove(&mut self, name: &str) -> bool {
        let n = self.parts.len();
        self.parts.retain(|p| p.name != name);
        n != self.parts.len()
    }
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.parts.iter().map(|p| p.name.as_str())
    }
    pub fn xml(&self, name: &str) -> Result<Document> {
        let b = self
            .get(name)
            .ok_or_else(|| Error::MissingPart(name.to_string()))?;
        Document::parse(b)
    }
    pub fn rels(&self, part: &str) -> Result<Relationships> {
        match self.get(&rels_path(part)) {
            Some(b) => Relationships::parse(b),
            None => Ok(Relationships::default()),
        }
    }
    pub fn set_rels(&mut self, part: &str, rels: &Relationships) {
        self.set(&rels_path(part), rels.to_bytes());
    }
    /// A part name not yet used, following `prefix{n}{suffix}` from n = 1.
    pub fn free_name(&self, prefix: &str, suffix: &str) -> String {
        (1..)
            .map(|n| format!("{prefix}{n}{suffix}"))
            .find(|n| !self.has(n))
            .unwrap()
    }
}

/// `ppt/slides/slide1.xml` -> `ppt/slides/_rels/slide1.xml.rels`
pub fn rels_path(part: &str) -> String {
    match part.rsplit_once('/') {
        Some((dir, file)) => format!("{dir}/_rels/{file}.rels"),
        None => format!("_rels/{part}.rels"),
    }
}

/// Resolve a relationship target relative to its source part.
pub fn resolve_target(source_part: &str, target: &str) -> String {
    if let Some(abs) = target.strip_prefix('/') {
        return abs.to_string();
    }
    let mut segs: Vec<&str> = match source_part.rsplit_once('/') {
        Some((dir, _)) => dir.split('/').collect(),
        None => Vec::new(),
    };
    for s in target.split('/') {
        match s {
            ".." => {
                segs.pop();
            }
            "." | "" => {}
            s => segs.push(s),
        }
    }
    segs.join("/")
}

/// Relative target from `source_part` to `target_part` (both package-absolute).
pub fn relative_target(source_part: &str, target_part: &str) -> String {
    let src_dir: Vec<&str> = source_part
        .rsplit_once('/')
        .map(|(d, _)| d.split('/').collect())
        .unwrap_or_default();
    let tgt: Vec<&str> = target_part.split('/').collect();
    let mut common = 0;
    while common < src_dir.len() && common + 1 < tgt.len() && src_dir[common] == tgt[common] {
        common += 1;
    }
    let mut out: Vec<String> = Vec::new();
    for _ in common..src_dir.len() {
        out.push("..".into());
    }
    for s in &tgt[common..] {
        out.push((*s).to_string());
    }
    out.join("/")
}

#[derive(Debug, Clone, PartialEq)]
pub struct Relationship {
    pub id: String,
    pub rel_type: String,
    pub target: String,
    pub external: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Relationships {
    pub items: Vec<Relationship>,
}

impl Relationships {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let d = Document::parse(bytes)?;
        let items = d
            .root
            .elements()
            .filter(|e| e.local() == "Relationship")
            .map(|e| Relationship {
                id: e.attr("Id").unwrap_or_default().to_string(),
                rel_type: e.attr("Type").unwrap_or_default().to_string(),
                target: e.attr("Target").unwrap_or_default().to_string(),
                external: e.attr("TargetMode") == Some("External"),
            })
            .collect();
        Ok(Self { items })
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut root = Element::new("Relationships").with_attr(
            "xmlns",
            "http://schemas.openxmlformats.org/package/2006/relationships",
        );
        for r in &self.items {
            let mut e = Element::new("Relationship")
                .with_attr("Id", &r.id)
                .with_attr("Type", &r.rel_type)
                .with_attr("Target", &r.target);
            if r.external {
                e.attrs.set("TargetMode", "External");
            }
            root.push(e);
        }
        Document::new(root).to_bytes()
    }
    pub fn get(&self, id: &str) -> Option<&Relationship> {
        self.items.iter().find(|r| r.id == id)
    }
    pub fn by_type<'a>(&'a self, t: &'a str) -> impl Iterator<Item = &'a Relationship> + 'a {
        self.items.iter().filter(move |r| r.rel_type == t)
    }
    pub fn next_id(&self) -> String {
        (1..)
            .map(|n| format!("rId{n}"))
            .find(|id| self.get(id).is_none())
            .unwrap()
    }
    pub fn add(&mut self, rel_type: &str, target: &str) -> String {
        let id = self.next_id();
        self.items.push(Relationship {
            id: id.clone(),
            rel_type: rel_type.into(),
            target: target.into(),
            external: false,
        });
        id
    }
}

/// `[Content_Types].xml` editing helpers (kept as a DOM so unknown bits survive).
pub struct ContentTypes {
    pub doc: Document,
}

impl ContentTypes {
    pub const PART: &'static str = "[Content_Types].xml";
    pub fn load(pkg: &Package) -> Result<Self> {
        Ok(Self {
            doc: pkg.xml(Self::PART)?,
        })
    }
    pub fn store(&self, pkg: &mut Package) {
        pkg.set(Self::PART, self.doc.to_bytes());
    }
    pub fn set_override(&mut self, part: &str, content_type: &str) {
        let pn = format!("/{part}");
        self.remove_override(part);
        self.doc.root.push(
            Element::new("Override")
                .with_attr("PartName", pn)
                .with_attr("ContentType", content_type),
        );
    }
    pub fn remove_override(&mut self, part: &str) {
        let pn = format!("/{part}");
        self.doc.root.children.retain(|n| match n {
            crate::xml::Node::Element(e) => {
                !(e.local() == "Override" && e.attr("PartName") == Some(pn.as_str()))
            }
            _ => true,
        });
    }
    /// Content type of a part (Override, else Default by extension).
    pub fn content_type_of(&self, part: &str) -> Option<String> {
        let pn = format!("/{part}");
        if let Some(o) = self
            .doc
            .root
            .elements()
            .find(|e| e.local() == "Override" && e.attr("PartName") == Some(pn.as_str()))
        {
            return o.attr("ContentType").map(str::to_string);
        }
        let ext = part.rsplit('.').next()?;
        self.doc
            .root
            .elements()
            .find(|e| {
                e.local() == "Default"
                    && e.attr("Extension").map(|x| x.eq_ignore_ascii_case(ext)) == Some(true)
            })
            .and_then(|e| e.attr("ContentType"))
            .map(str::to_string)
    }
    pub fn has_override(&self, part: &str) -> bool {
        let pn = format!("/{part}");
        self.doc
            .root
            .elements()
            .any(|e| e.local() == "Override" && e.attr("PartName") == Some(pn.as_str()))
    }
    pub fn ensure_default(&mut self, ext: &str, content_type: &str) {
        let has = self.doc.root.elements().any(|e| {
            e.local() == "Default"
                && e.attr("Extension").map(|x| x.eq_ignore_ascii_case(ext)) == Some(true)
        });
        if !has {
            self.doc.root.children.insert(
                0,
                crate::xml::Node::Element(
                    Element::new("Default")
                        .with_attr("Extension", ext)
                        .with_attr("ContentType", content_type),
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_resolve_both_ways() {
        assert_eq!(
            resolve_target("ppt/slides/slide1.xml", "../media/image1.png"),
            "ppt/media/image1.png"
        );
        assert_eq!(
            resolve_target("ppt/presentation.xml", "slides/slide2.xml"),
            "ppt/slides/slide2.xml"
        );
        assert_eq!(
            resolve_target("_rels/.rels", "/ppt/presentation.xml"),
            "ppt/presentation.xml"
        );
        assert_eq!(
            relative_target("ppt/slides/slide1.xml", "ppt/slideLayouts/slideLayout2.xml"),
            "../slideLayouts/slideLayout2.xml"
        );
        assert_eq!(
            relative_target("ppt/presentation.xml", "ppt/slides/slide3.xml"),
            "slides/slide3.xml"
        );
        assert_eq!(
            rels_path("ppt/slides/slide1.xml"),
            "ppt/slides/_rels/slide1.xml.rels"
        );
    }

    #[test]
    fn zip_roundtrip_keeps_bytes_and_order() {
        let mut p = Package::default();
        p.set("[Content_Types].xml", b"<Types/>".to_vec());
        p.set("ppt/media/image1.png", vec![1, 2, 3]);
        p.parts[1].stored = true;
        let b = p.to_bytes().unwrap();
        let q = Package::from_bytes(&b).unwrap();
        assert_eq!(q.parts, p.parts);
    }
}
