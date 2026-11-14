//! A lens: what the draft interview cares about.
//!
//! The interview engine knows nothing about what makes a paper good; the
//! lens tells it. A lens is `lens/<name>.toml` in the workspace (the user's
//! private repository), with an optional long guide in Markdown that goes
//! into every interview prompt as is. It defines:
//!
//! - `scale`: the depth levels an answer or a sentence is graded on
//!   (index 0 = lowest);
//! - `flag`s: extra marks an item can earn (e.g. "origin": how the author
//!   came to the idea);
//! - `mode`s: how an item is treated (asked only to confirm, never asked,
//!   or asked down to a depth);
//! - `target`s: what an item of a given role needs before it counts as done;
//! - `probe`s: question types the model may use when something is missing;
//! - `stop`: when a question group ends without everything done.
//!
//! Whether an item is done is decided here, in code, from the lens; the model
//! only grades answers.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// The lens used when the workspace has none: two levels, "has a reason or
/// not", and one probe. Kept neutral on purpose.
pub const DEFAULT_LENS: &str = include_str!("default_lens.toml");

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Lens {
    pub name: String,
    /// Markdown file next to the lens (read by `load`), put into prompts.
    #[serde(default)]
    pub guide: Option<String>,
    /// The guide's text after loading.
    #[serde(default, skip_deserializing)]
    pub guide_text: String,
    pub scale: Scale,
    #[serde(default, rename = "flag")]
    pub flags: Vec<Flag>,
    #[serde(default, rename = "mode")]
    pub modes: Vec<Mode>,
    #[serde(default, rename = "target")]
    pub targets: Vec<Target>,
    #[serde(default, rename = "probe")]
    pub probes: Vec<Probe>,
    #[serde(default)]
    pub stop: Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Scale {
    /// What is graded, e.g. "主觀層級".
    pub label: String,
    pub levels: Vec<Level>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Level {
    pub id: String,
    pub label: String,
    /// What an answer at this level looks like (for the model).
    #[serde(default)]
    pub means: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Flag {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub means: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Ask {
    /// Ask until the item reaches its depth.
    #[default]
    Deep,
    /// Only confirm that this is the usual practice; never ask why.
    Confirm,
    /// Never ask (e.g. explaining variables, defining terms).
    Never,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Mode {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub means: String,
    /// Typical members, to help the model sort items.
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub ask: Ask,
    /// Depth an item in this mode must reach (when `ask` is deep).
    #[serde(default)]
    pub min_depth: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Target {
    /// Item role this applies to, e.g. "core-contribution", "key-result".
    pub role: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub min_depth: usize,
    /// Flags the item must carry.
    #[serde(default)]
    pub flags: Vec<String>,
    /// The item must be backed by a number (a result).
    #[serde(default)]
    pub number: bool,
    /// Depth worth asking for once the minimum is met (never blocks).
    #[serde(default)]
    pub prefer_depth: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Probe {
    pub id: String,
    /// What has to be missing for the probe to apply: "mode", "depth",
    /// a flag id, or "number".
    pub when_missing: String,
    /// Limit to items of these roles (empty = any).
    #[serde(default)]
    pub roles: Vec<String>,
    /// The question, as a model hint (it may rephrase).
    pub ask: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Stop {
    /// Answers per group before the interview offers to move on.
    #[serde(default = "default_turns")]
    pub max_turns_per_group: usize,
    /// Offer 2–4 options when the author is unsure.
    #[serde(default = "yes")]
    pub options_when_unsure: bool,
    /// Answers per group by section, below `max_turns_per_group` (by importance: the details the
    /// record covers get fewer than the motive, the reading of results and the discussion).
    /// A section not listed gets `max_turns_per_group`.
    #[serde(default = "default_section_turns")]
    pub section_turns: std::collections::BTreeMap<String, usize>,
}

fn default_section_turns() -> std::collections::BTreeMap<String, usize> {
    [("Methods", 3), ("Results", 4), ("Conclusion", 2)].into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

impl Stop {
    /// A group's budget: its section's, or the full one (`full`: it holds what matters most).
    pub fn budget(&self, section: &str, full: bool) -> usize {
        match self.section_turns.get(section) {
            Some(n) if !full => (*n).clamp(1, self.max_turns_per_group.max(1)),
            _ => self.max_turns_per_group,
        }
    }
}

fn default_turns() -> usize {
    8
}
fn yes() -> bool {
    true
}

impl Default for Stop {
    fn default() -> Self {
        Stop { max_turns_per_group: default_turns(), options_when_unsure: true, section_turns: default_section_turns() }
    }
}

/// What an item still lacks under a lens.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "what")]
pub enum Missing {
    /// Not yet sorted into a mode.
    Mode,
    /// Needs this depth (index into the scale).
    Depth(usize),
    Flag(String),
    Number,
    /// A pending result the author has not explained yet (what, why, expectation, if not).
    Explain,
}

/// The parts of an item a lens judges.
pub trait Graded {
    fn role(&self) -> &str;
    fn mode(&self) -> Option<&str>;
    fn depth(&self) -> Option<usize>;
    fn has_flag(&self, f: &str) -> bool;
    fn has_number(&self) -> bool;
}

impl Lens {
    /// Parse a lens and check that its references hold together.
    pub fn parse(src: &str) -> Result<Lens, String> {
        let lens: Lens = toml::from_str(src).map_err(|e| format!("lens: {e}"))?;
        lens.validate()?;
        Ok(lens)
    }

    pub fn builtin() -> Lens {
        Lens::parse(DEFAULT_LENS).expect("bundled default lens is valid")
    }

    /// `lens/<name>.toml` in the workspace, with its guide; the built-in lens
    /// when the file does not exist. A file that exists but is broken is an
    /// error (the author should see it, not silently get the default).
    pub fn load(workspace: &Path, name: &str) -> Result<Lens, String> {
        if !crate::paper::safe_name(name) {
            return Err("invalid lens name".into());
        }
        let dir = workspace.join("lens");
        let file = dir.join(format!("{name}.toml"));
        let Ok(src) = std::fs::read_to_string(&file) else {
            return Ok(Lens::builtin());
        };
        let mut lens = Lens::parse(&src).map_err(|e| format!("{}: {e}", file.display()))?;
        if let Some(g) = lens.guide.as_deref().filter(|g| crate::paper::safe_name(g.trim_end_matches(".md"))) {
            lens.guide_text = std::fs::read_to_string(dir.join(g)).unwrap_or_default();
        }
        Ok(lens)
    }

    /// Lens names available in the workspace (file stems under `lens/`).
    pub fn available(workspace: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(workspace.join("lens"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".toml").map(str::to_string))
            .filter(|n| crate::paper::safe_name(n))
            .collect();
        v.sort();
        v
    }

    fn validate(&self) -> Result<(), String> {
        let n = self.scale.levels.len();
        if n < 2 {
            return Err("scale needs at least two levels".into());
        }
        let mut seen = std::collections::HashSet::new();
        for m in &self.modes {
            if !seen.insert(m.id.as_str()) {
                return Err(format!("duplicate mode {}", m.id));
            }
            if m.min_depth >= n {
                return Err(format!("mode {}: min_depth {} is beyond the scale", m.id, m.min_depth));
            }
        }
        if self.modes.is_empty() {
            return Err("at least one mode is needed".into());
        }
        let flags: Vec<&str> = self.flags.iter().map(|f| f.id.as_str()).collect();
        for t in &self.targets {
            if t.min_depth >= n || t.prefer_depth.is_some_and(|p| p >= n) {
                return Err(format!("target {}: depth beyond the scale", t.role));
            }
            if let Some(f) = t.flags.iter().find(|f| !flags.contains(&f.as_str())) {
                return Err(format!("target {}: unknown flag {f}", t.role));
            }
        }
        for p in &self.probes {
            let ok = matches!(p.when_missing.as_str(), "mode" | "depth" | "number") || flags.contains(&p.when_missing.as_str());
            if !ok {
                return Err(format!("probe {}: when_missing must be mode, depth, number or a flag", p.id));
            }
        }
        Ok(())
    }

    pub fn mode(&self, id: &str) -> Option<&Mode> {
        self.modes.iter().find(|m| m.id == id)
    }

    pub fn target(&self, role: &str) -> Option<&Target> {
        self.targets.iter().find(|t| t.role == role)
    }

    /// Level label for a depth, e.g. "L2 潛在因子".
    pub fn level_name(&self, depth: usize) -> String {
        self.scale.levels.get(depth).map(|l| format!("{} {}", l.id, l.label)).unwrap_or_else(|| depth.to_string())
    }

    /// What `item` still lacks. Empty = done.
    pub fn missing<G: Graded + ?Sized>(&self, item: &G) -> Vec<Missing> {
        let Some(mode) = item.mode().and_then(|m| self.mode(m)) else {
            return vec![Missing::Mode];
        };
        match mode.ask {
            Ask::Never | Ask::Confirm => return vec![],
            Ask::Deep => {}
        }
        let target = self.target(item.role());
        let need = mode.min_depth.max(target.map_or(0, |t| t.min_depth));
        let mut out = vec![];
        if item.depth().is_none_or(|d| d < need) {
            out.push(Missing::Depth(need));
        }
        if let Some(t) = target {
            out.extend(t.flags.iter().filter(|f| !item.has_flag(f)).map(|f| Missing::Flag(f.clone())));
            if t.number && !item.has_number() {
                out.push(Missing::Number);
            }
        }
        out
    }

    /// A depth worth asking for although the item is done.
    pub fn preferred<G: Graded + ?Sized>(&self, item: &G) -> Option<usize> {
        let p = self.target(item.role())?.prefer_depth?;
        (item.depth().unwrap_or(0) < p && self.missing(item).is_empty()).then_some(p)
    }

    /// Probes that fit what an item lacks.
    pub fn probes_for(&self, role: &str, missing: &[Missing]) -> Vec<&Probe> {
        self.probes
            .iter()
            .filter(|p| p.roles.is_empty() || p.roles.iter().any(|r| r == role))
            .filter(|p| {
                missing.iter().any(|m| match m {
                    Missing::Mode => p.when_missing == "mode",
                    Missing::Depth(_) => p.when_missing == "depth",
                    Missing::Number => p.when_missing == "number",
                    Missing::Flag(f) => &p.when_missing == f,
                    Missing::Explain => p.when_missing == "explain",
                })
            })
            .collect()
    }

    /// The lens as prompt text: scale, flags, modes, targets, then the guide.
    pub fn describe(&self) -> String {
        let mut s = format!("## Lens \"{}\"\n\n### {} (depth scale, lowest first)\n", self.name, self.scale.label);
        for (i, l) in self.scale.levels.iter().enumerate() {
            s += &format!("- {i} = {} {}: {}\n", l.id, l.label, l.means);
        }
        if !self.flags.is_empty() {
            s += "\n### Flags\n";
            for f in &self.flags {
                s += &format!("- {} ({}): {}\n", f.id, f.label, f.means);
            }
        }
        s += "\n### Modes (how an item is treated)\n";
        for m in &self.modes {
            let ask = match m.ask {
                Ask::Deep => format!("ask until depth >= {}", m.min_depth),
                Ask::Confirm => "only confirm it is the usual practice; never ask why".into(),
                Ask::Never => "never ask; no motive needed".into(),
            };
            s += &format!("- {} ({}): {} — {ask}", m.id, m.label, m.means);
            if !m.examples.is_empty() {
                s += &format!(" e.g. {}", m.examples.join(", "));
            }
            s += "\n";
        }
        if !self.targets.is_empty() {
            s += "\n### Targets by role\n";
            for t in &self.targets {
                let mut need = vec![format!("depth >= {}", t.min_depth)];
                need.extend(t.flags.iter().map(|f| format!("flag {f}")));
                if t.number {
                    need.push("backed by a number".into());
                }
                if let Some(p) = t.prefer_depth {
                    need.push(format!("depth {p} welcome"));
                }
                s += &format!("- {} {}: {}\n", t.role, t.label, need.join(", "));
            }
        }
        if !self.guide_text.trim().is_empty() {
            s += "\n### The author's own guide\n";
            s += self.guide_text.trim();
            s += "\n";
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct It {
        role: &'static str,
        mode: Option<&'static str>,
        depth: Option<usize>,
        flags: Vec<&'static str>,
        number: bool,
    }
    impl Graded for It {
        fn role(&self) -> &str {
            self.role
        }
        fn mode(&self) -> Option<&str> {
            self.mode
        }
        fn depth(&self) -> Option<usize> {
            self.depth
        }
        fn has_flag(&self, f: &str) -> bool {
            self.flags.contains(&f)
        }
        fn has_number(&self) -> bool {
            self.number
        }
    }
    fn it(role: &'static str, mode: Option<&'static str>, depth: Option<usize>) -> It {
        It { role, mode, depth, flags: vec![], number: false }
    }

    const FIVE: &str = r#"
name = "five"
[scale]
label = "depth"
levels = [
  { id = "L0", label = "fact" }, { id = "L1", label = "reading" }, { id = "L2", label = "factor" },
  { id = "L3", label = "relation" }, { id = "L4", label = "overturn" },
]
[[flag]]
id = "origin"
label = "how the idea came"
[[mode]]
id = "comply"
label = "usual practice"
ask = "confirm"
[[mode]]
id = "exposition"
label = "background"
ask = "never"
[[mode]]
id = "custom"
label = "own thinking"
min_depth = 2
[[target]]
role = "core-contribution"
min_depth = 3
flags = ["origin"]
prefer_depth = 4
[[target]]
role = "key-result"
min_depth = 2
number = true
[[probe]]
id = "origin"
when_missing = "origin"
roles = ["core-contribution"]
ask = "What did you first notice?"
[[probe]]
id = "why"
when_missing = "depth"
ask = "Why this and not the obvious choice?"
"#;

    #[test]
    fn builtin_lens_is_neutral_and_valid() {
        let l = Lens::builtin();
        assert_eq!(l.scale.levels.len(), 2);
        assert!(l.describe().contains("Modes"));
    }

    #[test]
    fn modes_and_targets_decide_what_is_missing() {
        let l = Lens::parse(FIVE).unwrap();
        // Unsorted: the only question is the mode.
        assert_eq!(l.missing(&it("design", None, Some(4))), vec![Missing::Mode]);
        // "BCE, no plan to try others": usual practice, done without a why.
        assert!(l.missing(&it("design", Some("comply"), None)).is_empty());
        // Explaining variables: never asked.
        assert!(l.missing(&it("design", Some("exposition"), Some(0))).is_empty());
        // Own design decision: needs a task-specific reason (depth 2).
        assert_eq!(l.missing(&it("design", Some("custom"), Some(1))), vec![Missing::Depth(2)]);
        assert!(l.missing(&it("design", Some("custom"), Some(2))).is_empty());
        // The core contribution needs depth 3 and the origin story.
        let core = It { role: "core-contribution", mode: Some("custom"), depth: Some(3), flags: vec![], number: false };
        assert_eq!(l.missing(&core), vec![Missing::Flag("origin".into())]);
        let core = It { flags: vec!["origin"], ..core };
        assert!(l.missing(&core).is_empty());
        assert_eq!(l.preferred(&core), Some(4), "an overturn is welcome, never required");
        // A key result needs its number.
        let res = It { role: "key-result", mode: Some("custom"), depth: Some(2), flags: vec![], number: false };
        assert_eq!(l.missing(&res), vec![Missing::Number]);
    }

    #[test]
    fn probes_match_what_is_missing() {
        let l = Lens::parse(FIVE).unwrap();
        let p = l.probes_for("core-contribution", &[Missing::Flag("origin".into()), Missing::Depth(3)]);
        assert_eq!(p.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["origin", "why"]);
        let p = l.probes_for("design", &[Missing::Flag("origin".into())]);
        assert!(p.is_empty(), "the origin probe is only for the core contribution");
    }

    #[test]
    fn the_public_template_lens_is_valid() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../private-example/PaperComposer/lens/default.toml");
        let l = Lens::parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(l, Lens::builtin(), "the template and the built-in lens say the same");
    }

    #[test]
    fn broken_lenses_are_rejected() {
        assert!(Lens::parse(&FIVE.replace("min_depth = 3", "min_depth = 9")).is_err());
        assert!(Lens::parse(&FIVE.replace("flags = [\"origin\"]", "flags = [\"nope\"]")).is_err());
        assert!(Lens::parse(&FIVE.replace("when_missing = \"depth\"", "when_missing = \"mood\"")).is_err());
    }

    #[test]
    fn loads_from_the_workspace_with_its_guide() {
        let ws = std::env::temp_dir().join(format!("pc-lens-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&ws);
        std::fs::create_dir_all(ws.join("lens")).unwrap();
        std::fs::write(ws.join("lens/five.toml"), format!("guide = \"five.md\"\n{FIVE}")).unwrap();
        std::fs::write(ws.join("lens/five.md"), "Ask about the timeline first.").unwrap();
        let l = Lens::load(&ws, "five").unwrap();
        assert!(l.describe().contains("Ask about the timeline first."));
        assert_eq!(Lens::available(&ws), ["five"]);
        assert_eq!(Lens::load(&ws, "absent").unwrap().name, Lens::builtin().name);
        assert!(Lens::load(&ws, "../x").is_err());
        let _ = std::fs::remove_dir_all(&ws);
    }
}
