//! Bring an existing LaTeX project in: unpack an upload (.zip, .tar.gz, a
//! gzipped or plain .tex) or an arXiv source into a temporary folder, then
//! `paper::import_paper` turns it into a paper.

use std::io::Read;
use std::path::Path;

/// Unpack `bytes` into `dest` (an empty folder).
pub fn unpack(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let err = |e: std::io::Error| format!("無法解開檔案：{e}");
    if bytes.starts_with(b"PK\x03\x04") {
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("無法解開 zip：{e}"))?;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
            // enclosed_name refuses absolute paths and `..`
            let Some(rel) = f.enclosed_name() else { continue };
            let out = dest.join(rel);
            if f.is_dir() {
                std::fs::create_dir_all(&out).map_err(err)?;
            } else {
                if let Some(p) = out.parent() {
                    std::fs::create_dir_all(p).map_err(err)?;
                }
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).map_err(err)?;
                std::fs::write(out, buf).map_err(err)?;
            }
        }
        return Ok(());
    }
    let data = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut v = Vec::new();
        flate2::read::GzDecoder::new(bytes).read_to_end(&mut v).map_err(err)?;
        v
    } else {
        bytes.to_vec()
    };
    if data.len() > 262 && &data[257..262] == b"ustar" {
        // tar's unpack keeps every entry inside dest
        tar::Archive::new(std::io::Cursor::new(data)).unpack(dest).map_err(err)
    } else if data.windows(14).any(|w| w == b"\\documentclass") {
        std::fs::write(dest.join("main.tex"), data).map_err(err)
    } else {
        Err("看不懂這個檔案：請上傳 .zip、.tar.gz 或 .tex".into())
    }
}

/// `1706.03762`, `1706.03762v7`, `hep-th/9901001`, or an arxiv.org URL.
pub fn arxiv_id(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s.split("arxiv.org/").nth(1).map(|r| r.split_once('/').map(|x| x.1).unwrap_or(r)).unwrap_or(s);
    let s = s.trim_start_matches("arXiv:").trim_start_matches("arxiv:").trim_end_matches(".pdf").trim_end_matches('/');
    let new = |t: &str| {
        let (a, b) = t.split_once('.')?;
        let b = b.split('v').next()?;
        (a.len() == 4 && a.chars().all(|c| c.is_ascii_digit()) && (4..=5).contains(&b.len()) && b.chars().all(|c| c.is_ascii_digit())).then_some(())
    };
    let old = |t: &str| {
        let (a, b) = t.split_once('/')?;
        (a.chars().all(|c| c.is_ascii_lowercase() || c == '-' || c == '.') && b.len() >= 7 && b[..7].chars().all(|c| c.is_ascii_digit())).then_some(())
    };
    (new(s).is_some() || old(s).is_some()).then(|| s.to_string())
}

/// The source archive arXiv serves for a paper.
pub async fn fetch_arxiv(id: &str) -> Result<Vec<u8>, String> {
    let base = std::env::var("PAPERCOMPOSER_ARXIV_BASE").unwrap_or_else(|_| "https://arxiv.org".into());
    let res = reqwest::Client::new()
        .get(format!("{base}/e-print/{id}"))
        .header("User-Agent", "PaperComposer (academic writing tool)")
        .send()
        .await
        .map_err(|e| format!("連不上 arXiv：{e}"))?;
    if !res.status().is_success() {
        return Err(format!("arXiv 找不到 {id}（{}）", res.status()));
    }
    let b = res.bytes().await.map_err(|e| e.to_string())?;
    if b.starts_with(b"%PDF") {
        return Err("這篇在 arXiv 只有 PDF，沒有 LaTeX 原始檔".into());
    }
    Ok(b.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_arxiv_ids() {
        assert_eq!(arxiv_id("1706.03762").as_deref(), Some("1706.03762"));
        assert_eq!(arxiv_id("https://arxiv.org/abs/1706.03762v7").as_deref(), Some("1706.03762v7"));
        assert_eq!(arxiv_id("https://arxiv.org/pdf/2401.12345.pdf").as_deref(), Some("2401.12345"));
        assert_eq!(arxiv_id("arXiv:hep-th/9901001").as_deref(), Some("hep-th/9901001"));
        assert_eq!(arxiv_id("my paper"), None);
        assert_eq!(arxiv_id("12.34"), None);
    }

    #[test]
    fn unpacks_plain_and_gzipped_tex() {
        let d = std::env::temp_dir().join(format!("pc-unpack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let tex = b"\\documentclass{article}\\begin{document}x\\end{document}";
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, tex).unwrap();
        unpack(&gz.finish().unwrap(), &d).unwrap();
        assert_eq!(std::fs::read(d.join("main.tex")).unwrap(), tex);
        assert!(unpack(b"hello", &d).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
