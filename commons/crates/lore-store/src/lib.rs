//! lore-store — data tiers and spaces (design §3).
//!
//! A *space* is a folder with a `space.toml` (id, name, kind personal|household,
//! members, assets backend). Apps mount several spaces at once ([`Spaces`]) and always
//! name the space they write to. Learning state is per member under
//! `progress/<member>/`. Images are content addressed ([`assets`]): sha256, stored as
//! `ab/abcd….ext` either in an iCloud Drive folder (personal) or in the space's own
//! git folder as webp with a size cap (household).
//!
//! Rules this crate helps enforce (the node-level checks live in `kb-core`):
//! nothing flows personal → household implicitly ([`check_flow`]); a household space
//! never stores references to personal node ids.

pub mod assets;
pub mod paths;
pub mod runner;
pub mod space;

pub use assets::{AssetRef, AssetStore, Backend};
pub use paths::{copy_missing, expand_home, free_path, valid_member, write_atomic};
pub use runner::{CommandOutput, CommandRunner, SystemRunner};
pub use space::{AssetsConfig, Space, SpaceConfig, SpaceKind, Spaces, check_flow};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("space.toml: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("space.toml: {0}")]
    TomlSer(#[from] toml::ser::Error),
    #[error("{0}")]
    Invalid(String),
    #[error("not found: {0}")]
    NotFound(String),
    /// Personal data must never reach a household space without an explicit share.
    #[error("not allowed: {0}")]
    Forbidden(String),
    #[error("image: {0}")]
    Image(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;
