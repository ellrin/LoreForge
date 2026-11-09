//! Spaces: `space.toml`, mounting, member-scoped paths.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::assets::{AssetStore, Backend};
use crate::paths::{valid_member, write_atomic};
use crate::runner::CommandRunner;
use crate::{Result, StoreError};

/// Default iCloud Drive folder for personal assets.
pub const DEFAULT_ICLOUD_ROOT: &str = "~/Library/Mobile Documents/com~apple~CloudDocs/KnowledgeBase/assets";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpaceKind {
    /// Only the author (own private repo, images in own iCloud Drive).
    Personal,
    /// Shared with family (shared private repo, images in git as webp).
    Household,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BackendKind {
    Icloud,
    Git,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AssetsConfig {
    /// `icloud` (default for personal) or `git` (default and required for household).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<BackendKind>,
    /// iCloud: the asset folder (`~/` allowed; default [`DEFAULT_ICLOUD_ROOT`]).
    /// git: folder relative to the space (default `assets`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// git backend: largest stored file in bytes (default 1.5 MB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
    /// git backend: longest image side in pixels (default 1600).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_dim: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpaceConfig {
    pub id: String,
    pub name: String,
    pub kind: SpaceKind,
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub assets: AssetsConfig,
}

impl SpaceConfig {
    pub fn backend(&self) -> BackendKind {
        self.assets.backend.unwrap_or(match self.kind {
            SpaceKind::Personal => BackendKind::Icloud,
            SpaceKind::Household => BackendKind::Git,
        })
    }

    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty() || !self.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(StoreError::Invalid(format!("space id `{}`: use letters, digits, - or _", self.id)));
        }
        if self.members.is_empty() {
            return Err(StoreError::Invalid(format!("space `{}` has no members", self.id)));
        }
        if let Some(m) = self.members.iter().find(|m| !valid_member(m)) {
            return Err(StoreError::Invalid(format!("member id `{m}`: use lowercase letters, digits, - or _")));
        }
        if self.kind == SpaceKind::Personal && self.members.len() > 1 {
            return Err(StoreError::Invalid(format!("personal space `{}` must have exactly one member", self.id)));
        }
        // The partner's iPad reads the shared repo through the GitHub API with another
        // Apple ID, so household images cannot live in someone's iCloud Drive.
        if self.kind == SpaceKind::Household && self.backend() == BackendKind::Icloud {
            return Err(StoreError::Invalid(format!("household space `{}` must keep assets in git", self.id)));
        }
        Ok(())
    }
}

/// One mounted space: a folder with `space.toml`.
#[derive(Debug, Clone)]
pub struct Space {
    pub root: PathBuf,
    pub config: SpaceConfig,
}

impl Space {
    /// Open `root/space.toml`.
    pub fn open(root: impl Into<PathBuf>) -> Result<Space> {
        let root = root.into();
        let file = root.join("space.toml");
        let text = std::fs::read_to_string(&file).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => StoreError::NotFound(file.display().to_string()),
            _ => e.into(),
        })?;
        let config: SpaceConfig = toml::from_str(&text)?;
        config.validate()?;
        Ok(Space { root, config })
    }

    /// Create a new space (writes `space.toml` and the standard folders). Never
    /// overwrites an existing `space.toml`.
    pub fn create(root: impl Into<PathBuf>, config: SpaceConfig) -> Result<Space> {
        config.validate()?;
        let root = root.into();
        let file = root.join("space.toml");
        if file.exists() {
            return Err(StoreError::Invalid(format!("{} already exists", file.display())));
        }
        write_atomic(&file, toml::to_string_pretty(&config)?.as_bytes())?;
        let space = Space { root, config };
        std::fs::create_dir_all(space.nodes_dir())?;
        for m in &space.config.members {
            std::fs::create_dir_all(space.root.join("progress").join(m))?;
        }
        Ok(space)
    }

    pub fn id(&self) -> &str {
        &self.config.id
    }
    pub fn kind(&self) -> SpaceKind {
        self.config.kind
    }
    pub fn is_household(&self) -> bool {
        self.config.kind == SpaceKind::Household
    }
    pub fn is_member(&self, member: &str) -> bool {
        self.config.members.iter().any(|m| m == member)
    }
    pub fn nodes_dir(&self) -> PathBuf {
        self.root.join("nodes")
    }
    pub fn trash_dir(&self) -> PathBuf {
        self.root.join("trash")
    }
    /// A file directly in the space (`kinds.toml`, `tags.json`, …).
    pub fn file(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// `progress/<member>/` — only for members of this space. In household mode each
    /// member commits only their own folder.
    pub fn progress_dir(&self, member: &str) -> Result<PathBuf> {
        if !valid_member(member) {
            return Err(StoreError::Invalid(format!("member id `{member}`")));
        }
        if !self.is_member(member) {
            return Err(StoreError::Forbidden(format!("`{member}` is not a member of space `{}`", self.id())));
        }
        Ok(self.root.join("progress").join(member))
    }

    /// The folder this space's assets live in.
    pub fn assets_root(&self) -> PathBuf {
        match self.config.backend() {
            BackendKind::Icloud => crate::paths::expand_home(self.config.assets.root.as_deref().unwrap_or(DEFAULT_ICLOUD_ROOT)),
            BackendKind::Git => self.root.join(self.config.assets.root.as_deref().unwrap_or("assets")),
        }
    }

    /// The asset store for this space. `runner` runs `brctl` for iCloud placeholders.
    pub fn assets(&self, runner: Arc<dyn CommandRunner>) -> AssetStore {
        let backend = match self.config.backend() {
            BackendKind::Icloud => Backend::Icloud,
            BackendKind::Git => Backend::Git {
                max_bytes: self.config.assets.max_bytes.unwrap_or(1_500_000),
                max_dim: self.config.assets.max_dim.unwrap_or(1600),
            },
        };
        AssetStore::new(self.assets_root(), backend, runner)
    }
}

/// Content may move between two spaces only by an explicit share (a copy). Moving
/// within one space is always fine.
pub fn check_flow(from: &Space, to: &Space, explicit: bool) -> Result<()> {
    if from.id() == to.id() || explicit {
        return Ok(());
    }
    Err(StoreError::Forbidden(format!(
        "content from {:?} space `{}` reaches {:?} space `{}` only through an explicit share",
        from.kind(),
        from.id(),
        to.kind(),
        to.id()
    )))
}

/// Several mounted spaces (personal, household…). Reads may span them; every write
/// names one.
#[derive(Debug, Clone, Default)]
pub struct Spaces {
    list: Vec<Space>,
}

impl Spaces {
    pub fn new() -> Spaces {
        Spaces::default()
    }

    /// Mount the space at `root`. Fails on a second space with the same id.
    pub fn mount(&mut self, root: impl AsRef<Path>) -> Result<&Space> {
        let space = Space::open(root.as_ref())?;
        self.add(space)
    }

    pub fn add(&mut self, space: Space) -> Result<&Space> {
        if self.get(space.id()).is_some() {
            return Err(StoreError::Invalid(format!("space `{}` is already mounted", space.id())));
        }
        self.list.push(space);
        Ok(self.list.last().expect("just pushed"))
    }

    pub fn get(&self, id: &str) -> Option<&Space> {
        self.list.iter().find(|s| s.id() == id)
    }

    pub fn require(&self, id: &str) -> Result<&Space> {
        self.get(id).ok_or_else(|| StoreError::NotFound(format!("space `{id}`")))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Space> {
        self.list.iter()
    }

    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// The first personal space whose member is `member`, else the first space.
    pub fn default_for(&self, member: &str) -> Option<&Space> {
        self.list.iter().find(|s| s.kind() == SpaceKind::Personal && s.is_member(member)).or(self.list.first())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn personal() -> SpaceConfig {
        SpaceConfig { id: "me".into(), name: "我的".into(), kind: SpaceKind::Personal, members: vec!["ellrin".into()], assets: AssetsConfig::default() }
    }
    fn household() -> SpaceConfig {
        SpaceConfig { id: "home".into(), name: "家".into(), kind: SpaceKind::Household, members: vec!["ellrin".into(), "daidai".into()], assets: AssetsConfig::default() }
    }

    #[test]
    fn parse_space_toml() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join("space.toml"),
            r#"id = "kb"
name = "知識庫"
kind = "personal"
members = ["ellrin"]
[assets]
backend = "icloud"
root = "/tmp/somewhere"
"#,
        )
        .unwrap();
        let s = Space::open(d.path()).unwrap();
        assert_eq!(s.kind(), SpaceKind::Personal);
        assert_eq!(s.config.backend(), BackendKind::Icloud);
        assert_eq!(s.assets_root(), PathBuf::from("/tmp/somewhere"));
        assert!(s.progress_dir("ellrin").unwrap().ends_with("progress/ellrin"));
        assert!(matches!(s.progress_dir("daidai"), Err(StoreError::Forbidden(_))));
        assert!(s.progress_dir("..").is_err());
    }

    #[test]
    fn defaults_and_rules() {
        let mut p = personal();
        assert_eq!(p.backend(), BackendKind::Icloud);
        assert_eq!(household().backend(), BackendKind::Git);
        let mut h = household();
        h.assets.backend = Some(BackendKind::Icloud);
        assert!(h.validate().is_err());
        p.members.push("daidai".into());
        assert!(p.validate().is_err());
        let d = tempfile::tempdir().unwrap();
        let hs = Space::create(d.path(), household()).unwrap();
        assert!(hs.assets_root().ends_with("assets"));
        assert!(d.path().join("progress/daidai").is_dir());
        assert!(Space::create(d.path(), household()).is_err(), "never overwrites space.toml");
    }

    #[test]
    fn mount_several_and_flow() {
        let d = tempfile::tempdir().unwrap();
        Space::create(d.path().join("p"), personal()).unwrap();
        Space::create(d.path().join("h"), household()).unwrap();
        let mut spaces = Spaces::new();
        spaces.mount(d.path().join("p")).unwrap();
        spaces.mount(d.path().join("h")).unwrap();
        assert!(spaces.mount(d.path().join("h")).is_err());
        assert_eq!(spaces.len(), 2);
        assert_eq!(spaces.default_for("ellrin").unwrap().id(), "me");
        let (p, h) = (spaces.get("me").unwrap(), spaces.get("home").unwrap());
        assert!(check_flow(p, h, false).is_err());
        assert!(check_flow(h, p, false).is_err());
        assert!(check_flow(p, h, true).is_ok());
        assert!(check_flow(p, p, false).is_ok());
    }
}
