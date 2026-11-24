//! What an app needs without opening the whole hub (it works with L-Brain closed): where the
//! hub is, the apps' declarations, and the items outside applications aligned to one of its
//! categories. (Stubs: filled by package K10; the signatures are frozen.)

use std::path::{Path, PathBuf};

use crate::Item;

/// The hub folder of the author's personal space: `<private>/KnowledgeBase/hub`.
pub fn hub_dir(home: &lore_home::Home) -> Option<PathBuf> {
    home.app_dir("KnowledgeBase").map(|d| d.join("hub"))
}

/// Every Lore app's declaration of its categories (`manifests/*.toml` of this crate).
pub fn builtin_manifests() -> Vec<&'static str> {
    vec![]
}

/// The items outside applications aligned to `category`, as the hub keeps them (never in the
/// app's own data). Empty when the hub or the category has none; never an error.
pub fn aligned(hub: &Path, category: &str) -> Vec<Item> {
    let _ = (hub, category);
    vec![]
}
