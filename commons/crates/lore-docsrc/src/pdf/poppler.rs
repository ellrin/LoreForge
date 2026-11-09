//! poppler command-line tools (`pdftotext`, `pdftoppm`, `pdfinfo`, `pdfimages`).
//! Paths are configurable; a missing tool is a clear [`Error::ToolMissing`].

use crate::error::{Error, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the poppler binaries live.
#[derive(Debug, Clone, PartialEq)]
pub struct Poppler {
    pub pdftotext: PathBuf,
    pub pdftoppm: PathBuf,
    pub pdfinfo: PathBuf,
    pub pdfimages: PathBuf,
}

const TOOLS: [&str; 4] = ["pdftotext", "pdftoppm", "pdfinfo", "pdfimages"];

fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}

impl Default for Poppler {
    fn default() -> Self {
        Poppler::discover()
    }
}

impl Poppler {
    /// `$LORE_POPPLER_DIR` → `/opt/homebrew/bin` → `/usr/local/bin` → `/usr/bin` → `PATH`.
    /// Tools that are not found keep their bare name (and fail clearly when used).
    pub fn discover() -> Poppler {
        let mut dirs: Vec<PathBuf> = vec![];
        // YM_POPPLER_DIR: the name before the crates were renamed, still honoured.
        if let Some(d) = std::env::var_os("LORE_POPPLER_DIR").or_else(|| std::env::var_os("YM_POPPLER_DIR")) {
            dirs.push(PathBuf::from(d));
        }
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].map(PathBuf::from));
        let find = |name: &str| {
            dirs.iter()
                .map(|d| d.join(name))
                .find(|p| p.is_file())
                .or_else(|| on_path(name))
                .unwrap_or_else(|| PathBuf::from(name))
        };
        Poppler {
            pdftotext: find(TOOLS[0]),
            pdftoppm: find(TOOLS[1]),
            pdfinfo: find(TOOLS[2]),
            pdfimages: find(TOOLS[3]),
        }
    }

    /// All four tools from one folder.
    pub fn with_dir(dir: &Path) -> Poppler {
        Poppler {
            pdftotext: dir.join(TOOLS[0]),
            pdftoppm: dir.join(TOOLS[1]),
            pdfinfo: dir.join(TOOLS[2]),
            pdfimages: dir.join(TOOLS[3]),
        }
    }

    fn check(path: &Path, name: &str) -> Result<()> {
        if path.is_file() {
            Ok(())
        } else {
            Err(Error::ToolMissing {
                tool: name.into(),
                hint: format!("找過 {}", path.display()),
            })
        }
    }

    /// The tools PDF extraction needs (`pdftotext`, `pdftoppm`, `pdfinfo`).
    pub fn require(&self) -> Result<()> {
        Self::check(&self.pdftotext, "pdftotext")?;
        Self::check(&self.pdftoppm, "pdftoppm")?;
        Self::check(&self.pdfinfo, "pdfinfo")
    }

    pub fn available(&self) -> bool {
        self.require().is_ok()
    }

    fn run(&self, tool: &Path, name: &str, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>> {
        Self::check(tool, name)?;
        let out = Command::new(tool).args(args).output()?;
        if !out.status.success() {
            return Err(Error::ToolFailed {
                tool: name.into(),
                status: out.status.to_string(),
                stderr: String::from_utf8_lossy(&out.stderr)
                    .trim()
                    .chars()
                    .take(400)
                    .collect(),
            });
        }
        Ok(out.stdout)
    }

    /// `pdfinfo` key/value pairs.
    pub fn info(&self, pdf: &Path) -> Result<PdfInfo> {
        let out = self.run(&self.pdfinfo, "pdfinfo", &[pdf.as_os_str()])?;
        Ok(PdfInfo::parse(&String::from_utf8_lossy(&out)))
    }

    /// `pdftotext -bbox-layout` XHTML for the whole document.
    pub fn bbox_layout(&self, pdf: &Path) -> Result<String> {
        let out = self.run(
            &self.pdftotext,
            "pdftotext",
            &[
                "-bbox-layout".as_ref(),
                "-enc".as_ref(),
                "UTF-8".as_ref(),
                pdf.as_os_str(),
                "-".as_ref(),
            ],
        )?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// Render one page (1-based) to `<out_prefix>.png`.
    pub fn render_page(
        &self,
        pdf: &Path,
        page: u32,
        dpi: u32,
        gray: bool,
        out_prefix: &Path,
    ) -> Result<PathBuf> {
        let p = page.to_string();
        let r = dpi.to_string();
        let mut args: Vec<&std::ffi::OsStr> = vec![
            "-f".as_ref(),
            p.as_ref(),
            "-l".as_ref(),
            p.as_ref(),
            "-r".as_ref(),
            r.as_ref(),
            "-png".as_ref(),
            "-singlefile".as_ref(),
        ];
        if gray {
            args.push("-gray".as_ref());
        }
        args.push(pdf.as_os_str());
        args.push(out_prefix.as_os_str());
        self.run(&self.pdftoppm, "pdftoppm", &args)?;
        let mut png = out_prefix.as_os_str().to_owned();
        png.push(".png");
        Ok(PathBuf::from(png))
    }

    /// `pdfimages -list`: embedded raster images per page.
    pub fn images(&self, pdf: &Path) -> Result<Vec<ImageInfo>> {
        let out = self.run(
            &self.pdfimages,
            "pdfimages",
            &["-list".as_ref(), pdf.as_os_str()],
        )?;
        Ok(ImageInfo::parse_list(&String::from_utf8_lossy(&out)))
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PdfInfo {
    pub pages: u32,
    pub title: Option<String>,
    pub author: Option<String>,
    /// First page size in points.
    pub page_size: Option<(f64, f64)>,
}

impl PdfInfo {
    pub fn parse(s: &str) -> PdfInfo {
        let mut info = PdfInfo::default();
        for line in s.lines() {
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "Pages" => info.pages = v.parse().unwrap_or(0),
                "Title" if !v.is_empty() => info.title = Some(v.to_string()),
                "Author" if !v.is_empty() => info.author = Some(v.to_string()),
                "Page size" => {
                    let nums: Vec<f64> = v
                        .split_whitespace()
                        .filter_map(|w| w.parse().ok())
                        .collect();
                    if nums.len() >= 2 {
                        info.page_size = Some((nums[0], nums[1]));
                    }
                }
                _ => {}
            }
        }
        info
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageInfo {
    pub page: u32,
    pub width: u32,
    pub height: u32,
    /// `image`, `mask`, `smask`, `stencil`.
    pub kind: String,
}

impl ImageInfo {
    pub fn parse_list(s: &str) -> Vec<ImageInfo> {
        s.lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.split_whitespace().collect();
                if f.len() < 5 {
                    return None;
                }
                Some(ImageInfo {
                    page: f[0].parse().ok()?,
                    kind: f[2].to_string(),
                    width: f[3].parse().ok()?,
                    height: f[4].parse().ok()?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_tools_fail_clearly() {
        let p = Poppler::with_dir(Path::new("/nonexistent-poppler"));
        let e = p.info(Path::new("x.pdf")).unwrap_err();
        assert!(matches!(e, Error::ToolMissing { ref tool, .. } if tool == "pdfinfo"));
        assert!(e.to_string().contains("brew install poppler"));
        assert!(!p.available());
    }

    #[test]
    fn parses_pdfinfo_and_image_list() {
        let i = PdfInfo::parse(
            "Title:           A Paper\nAuthor:          \nPages:           10\nPage size:       595.276 x 793.701 pts\n",
        );
        assert_eq!(i.pages, 10);
        assert_eq!(i.title.as_deref(), Some("A Paper"));
        assert_eq!(i.author, None);
        assert_eq!(i.page_size, Some((595.276, 793.701)));
        let l = ImageInfo::parse_list(
            "page   num  type   width height color comp bpc  enc interp  object ID x-ppi y-ppi size ratio\n--------\n   3     0 image    1200   800  rgb     3   8  jpeg   no        12  0   300   300  100K 3.5%\n",
        );
        assert_eq!(
            l,
            vec![ImageInfo {
                page: 3,
                width: 1200,
                height: 800,
                kind: "image".into()
            }]
        );
    }
}
