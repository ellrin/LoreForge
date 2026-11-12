//! Time axes and download origin, each with its source.
//!
//! - file system: birth time and mtime;
//! - macOS xattrs: `com.apple.metadata:kMDItemWhereFroms` (binary plist list:
//!   download URL, then referrer), `com.apple.quarantine`
//!   (`flags;hex unix seconds;agent;uuid` → download time and app),
//!   `com.apple.lastuseddate#PS` (16 bytes: i64 seconds, i64 nanoseconds,
//!   little endian → last opened);
//! - documents: PDF Info `/CreationDate` `/ModDate` (lopdf's metadata reader,
//!   size-capped and panic-guarded, then a raw byte scan as fallback);
//!   OOXML `docProps/core.xml` `dcterms:created` / `dcterms:modified`.
//!
//! Everything is tolerant: a broken file gives no date, never an error.
//! Personal fields (authors, `cp:lastModifiedBy`) are never read.

use crate::record::{Download, TimePoint, axis};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub const XATTR_WHERE_FROMS: &str = "com.apple.metadata:kMDItemWhereFroms";
pub const XATTR_QUARANTINE: &str = "com.apple.quarantine";
pub const XATTR_LAST_USED: &str = "com.apple.lastuseddate#PS";

/// PDFs larger than this are not parsed with lopdf (it reads the whole file).
pub const PDF_PARSE_MAX: u64 = 200 << 20;
/// Bytes scanned at each end of a PDF by the fallback.
const PDF_SCAN_LEN: u64 = 2 << 20;

fn system_time(t: std::time::SystemTime) -> Option<DateTime<Utc>> {
    let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    Utc.timestamp_opt(d.as_secs() as i64, d.subsec_nanos()).single()
}

/// Birth time and mtime from `symlink_metadata`.
pub fn fs_times(md: &std::fs::Metadata) -> Vec<TimePoint> {
    let mut v = Vec::new();
    if let Some(t) = md.created().ok().and_then(system_time) {
        v.push(TimePoint::new(axis::FS_CREATED, t, "fs"));
    }
    if let Some(t) = md.modified().ok().and_then(system_time) {
        v.push(TimePoint::new(axis::FS_MODIFIED, t, "fs"));
    }
    v
}

/// A not-following xattr read (`None` when absent or unsupported).
pub fn read_xattr(path: &Path, name: &str) -> Option<Vec<u8>> {
    #[cfg(unix)]
    {
        xattr::get(path, name).ok().flatten()
    }
    #[cfg(not(unix))]
    {
        let _ = (path, name);
        None
    }
}

/// Download origin and the xattr time axes of one file.
pub fn xattr_info(path: &Path) -> (Vec<TimePoint>, Download) {
    let mut times = Vec::new();
    let mut dl = Download::default();
    if let Some(b) = read_xattr(path, XATTR_WHERE_FROMS) {
        let list = parse_where_froms(&b);
        let mut it = list.into_iter();
        if let Some(u) = it.next() {
            dl.urls.push(u);
        }
        dl.referrer = it.next();
    }
    if let Some(b) = read_xattr(path, XATTR_QUARANTINE)
        && let Some((t, agent)) = parse_quarantine(&b)
    {
        if let Some(t) = t {
            times.push(TimePoint::new(axis::DOWNLOADED, t, "xattr:com.apple.quarantine"));
        }
        dl.agent = agent;
    }
    if let Some(b) = read_xattr(path, XATTR_LAST_USED)
        && let Some(t) = parse_last_used(&b)
    {
        times.push(TimePoint::new(axis::LAST_OPENED, t, "xattr:com.apple.lastuseddate#PS"));
    }
    (times, dl)
}

/// `kMDItemWhereFroms`: a (usually binary) plist array of strings.
pub fn parse_where_froms(b: &[u8]) -> Vec<String> {
    match plist::Value::from_reader(std::io::Cursor::new(b)) {
        Ok(plist::Value::Array(a)) => a.into_iter().filter_map(|v| v.into_string()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
        Ok(plist::Value::String(s)) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => vec![],
    }
}

/// `com.apple.quarantine` = `flags;hexsecs;agent;uuid` (fields may be
/// missing or empty) → (download time, agent).
pub fn parse_quarantine(b: &[u8]) -> Option<(Option<DateTime<Utc>>, Option<String>)> {
    let s = String::from_utf8_lossy(b);
    let s = s.trim_end_matches('\0');
    let mut f = s.split(';');
    let _flags = f.next()?;
    let t = f.next().and_then(|h| i64::from_str_radix(h.trim(), 16).ok()).filter(|&t| t > 0).and_then(|t| Utc.timestamp_opt(t, 0).single());
    let agent = f.next().map(str::trim).filter(|a| !a.is_empty()).map(str::to_string);
    Some((t, agent))
}

/// `com.apple.lastuseddate#PS`: struct timespec, two little-endian i64.
pub fn parse_last_used(b: &[u8]) -> Option<DateTime<Utc>> {
    if b.len() < 16 {
        return None;
    }
    let secs = i64::from_le_bytes(b[0..8].try_into().ok()?);
    let nanos = i64::from_le_bytes(b[8..16].try_into().ok()?);
    if secs <= 0 || !(0..1_000_000_000).contains(&nanos) {
        return None;
    }
    Utc.timestamp_opt(secs, nanos as u32).single()
}

/// A PDF date `D:YYYYMMDDHHmmSSOHH'mm'` (every part after the year optional,
/// `D:` optional) → UTC.
pub fn parse_pdf_date(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim().trim_start_matches('\u{feff}');
    let s = s.strip_prefix("D:").unwrap_or(s);
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() < 4 {
        return None;
    }
    let num = |a: usize, b: usize, d: u32| -> u32 { digits.get(a..b).and_then(|x| x.parse().ok()).unwrap_or(d) };
    let year: i32 = digits[0..4].parse().ok()?;
    if !(1980..=2100).contains(&year) {
        return None;
    }
    let date = NaiveDate::from_ymd_opt(year, num(4, 6, 1), num(6, 8, 1))?;
    let naive = date.and_hms_opt(num(8, 10, 0), num(10, 12, 0), num(12, 14, 0).min(59))?;
    let rest = &s[digits.len()..];
    let offset_secs = match rest.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let tz: String = rest[1..].chars().filter(|c| c.is_ascii_digit()).collect();
            let h: i32 = tz.get(0..2).and_then(|x| x.parse().ok()).unwrap_or(0);
            let m: i32 = tz.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(0);
            let o = h * 3600 + m * 60;
            if sign == '-' { -o } else { o }
        }
        _ => 0, // `Z` or none: treat as UTC
    };
    let off = chrono::FixedOffset::east_opt(offset_secs)?;
    Some(off.from_local_datetime(&naive).single()?.with_timezone(&Utc))
}

/// A W3CDTF date from `core.xml` (`2024-03-01T10:00:00Z`, or date only).
pub fn parse_w3c_date(s: &str) -> Option<DateTime<Utc>> {
    let s = s.trim();
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc)).filter(|t| t.timestamp() > 0);
    }
    if let Ok(t) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(Utc.from_utc_datetime(&t));
    }
    NaiveDate::parse_from_str(s.get(0..10)?, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)).map(|t| Utc.from_utc_datetime(&t))
}

/// `/CreationDate` and `/ModDate` of a PDF.
pub fn pdf_dates(path: &Path, size: u64) -> Vec<TimePoint> {
    let mut v = Vec::new();
    let (mut created, mut modified) = (None, None);
    if size <= PDF_PARSE_MAX {
        let p = path.to_path_buf();
        if let Ok(Ok(md)) = std::panic::catch_unwind(move || lopdf::Document::load_metadata(&p)) {
            created = md.creation_date.as_deref().and_then(parse_pdf_date);
            modified = md.modification_date.as_deref().and_then(parse_pdf_date);
        }
    }
    if created.is_none() && modified.is_none() {
        let bytes = pdf_ends(path, size);
        created = scan_pdf_key(&bytes, b"/CreationDate").and_then(|s| parse_pdf_date(&s));
        modified = scan_pdf_key(&bytes, b"/ModDate").and_then(|s| parse_pdf_date(&s));
    }
    if let Some(t) = created {
        v.push(TimePoint::new(axis::DOC_CREATED, t, "pdf:Info"));
    }
    if let Some(t) = modified {
        v.push(TimePoint::new(axis::DOC_MODIFIED, t, "pdf:Info"));
    }
    v
}

/// The last and first `PDF_SCAN_LEN` bytes (the Info dictionary is near one end).
fn pdf_ends(path: &Path, size: u64) -> Vec<u8> {
    let Ok(mut f) = std::fs::File::open(path) else { return vec![] };
    let mut out = Vec::new();
    if size > PDF_SCAN_LEN && f.seek(SeekFrom::Start(size - PDF_SCAN_LEN)).is_ok() {
        let _ = (&mut f).take(PDF_SCAN_LEN).read_to_end(&mut out);
        let _ = f.seek(SeekFrom::Start(0));
    }
    let _ = f.take(PDF_SCAN_LEN).read_to_end(&mut out);
    out
}

/// The literal (`(D:…)`) or hex (`<…>`, also UTF-16BE) string after `key`.
pub fn scan_pdf_key(bytes: &[u8], key: &[u8]) -> Option<String> {
    let mut from = 0;
    while let Some(i) = bytes[from..].windows(key.len()).position(|w| w == key) {
        let start = from + i + key.len();
        from = start;
        let rest = &bytes[start..bytes.len().min(start + 200)];
        let rest = &rest[rest.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(rest.len())..];
        match rest.first() {
            Some(b'(') => {
                let end = rest.iter().position(|&b| b == b')')?;
                let raw = &rest[1..end];
                let s = if raw.starts_with(&[0xfe, 0xff]) { utf16be(&raw[2..]) } else { String::from_utf8_lossy(raw).to_string() };
                return Some(s);
            }
            Some(b'<') => {
                let end = rest.iter().position(|&b| b == b'>')?;
                let hexs: String = String::from_utf8_lossy(&rest[1..end]).chars().filter(|c| c.is_ascii_hexdigit()).collect();
                let raw = hex::decode(&hexs).ok()?;
                let s = if raw.starts_with(&[0xfe, 0xff]) { utf16be(&raw[2..]) } else { String::from_utf8_lossy(&raw).to_string() };
                return Some(s);
            }
            _ => continue,
        }
    }
    None
}

fn utf16be(b: &[u8]) -> String {
    let u: Vec<u16> = b.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
    String::from_utf16_lossy(&u)
}

/// `dcterms:created` / `dcterms:modified` from an OOXML package.
pub fn ooxml_dates(path: &Path) -> Vec<TimePoint> {
    let mut v = Vec::new();
    let Some(xml) = read_core_xml(path) else { return v };
    let (created, modified) = core_xml_dates(&xml);
    if let Some(t) = created {
        v.push(TimePoint::new(axis::DOC_CREATED, t, "ooxml:docProps/core.xml"));
    }
    if let Some(t) = modified {
        v.push(TimePoint::new(axis::DOC_MODIFIED, t, "ooxml:docProps/core.xml"));
    }
    v
}

fn read_core_xml(path: &Path) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let mut z = zip::ZipArchive::new(f).ok()?;
    let entry = z.by_name("docProps/core.xml").ok()?;
    if entry.size() > 1 << 20 {
        return None;
    }
    let mut s = String::new();
    entry.take(1 << 20).read_to_string(&mut s).ok()?;
    Some(s)
}

/// (created, modified) from the text of `core.xml`.
pub fn core_xml_dates(xml: &str) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    use quick_xml::events::Event;
    let mut r = quick_xml::Reader::from_str(xml);
    let (mut created, mut modified) = (None, None);
    let mut current: Option<&'static str> = None;
    loop {
        match r.read_event() {
            Ok(Event::Start(s)) => {
                current = match s.name().as_ref() {
                    "dcterms:created" => Some("c"),
                    "dcterms:modified" => Some("m"),
                    _ => None,
                };
            }
            Ok(Event::Text(t)) => {
                if let Some(which) = current {
                    let txt = t.xml10_content().into_owned();
                    let d = parse_w3c_date(&txt);
                    if which == "c" { created = created.or(d) } else { modified = modified.or(d) }
                }
            }
            Ok(Event::End(_)) => current = None,
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (created, modified)
}
