//! Who presents: the cover's Speaker and Supervisor are picked from the people L-Paper keeps
//! (`profile/authors.csv` of its workspace, read from disk like `pc_papers` reads its papers;
//! L-Paper is never started). Only an id and a display name leave this file: the CSV also
//! holds e-mail addresses and ORCID ids, and `/api/todeck*` answers other local origins.
//! The last choice is remembered in this app's private workspace (`paper-deck.json`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Person {
    pub id: String,
    pub name: String,
}

/// One CSV record (quotes, doubled quotes and commas inside quotes).
fn record(line: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut quoted = false;
    let mut cs = line.chars().peekable();
    while let Some(c) = cs.next() {
        match c {
            '"' if quoted && cs.peek() == Some(&'"') => {
                cur.push('"');
                cs.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cur)),
            c => cur.push(c),
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

/// The people of an `authors.csv` (columns `id`, `given`, `family`; the others are not read).
pub fn parse(csv: &str) -> Vec<Person> {
    let mut lines = csv.lines().filter(|l| !l.trim().is_empty());
    let Some(head) = lines.next().map(|l| record(l.trim_start_matches('\u{FEFF}'))) else { return vec![] };
    let col = |n: &str| head.iter().position(|h| h == n);
    let (Some(id), Some(family)) = (col("id"), col("family")) else { return vec![] };
    let given = col("given");
    lines
        .map(record)
        .filter_map(|r| {
            let id = r.get(id)?.clone();
            let family = r.get(family)?.clone();
            let given = given.and_then(|g| r.get(g)).cloned().unwrap_or_default();
            let cjk = |s: &str| s.chars().any(|c| ('\u{3400}'..='\u{9FFF}').contains(&c));
            let name = if cjk(&family) { format!("{family}{given}") } else { [given, family].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ") };
            (!id.is_empty() && !name.is_empty()).then_some(Person { id, name })
        })
        .collect()
}

/// L-Paper's people file. `real`: found like L-Paper finds its workspace (its env var, its
/// config, lore-home); off (isolated servers, tests): only the given home's folder.
pub fn file(home: &lore_home::Home, real: bool) -> Option<PathBuf> {
    let ws = if real { super::source::pc_workspace(home) } else { home.app_dir("PaperComposer") };
    ws.map(|w| w.join("profile/authors.csv"))
}

pub fn load(file: Option<&Path>) -> Vec<Person> {
    file.and_then(|f| std::fs::read_to_string(f).ok()).map(|t| parse(&t)).unwrap_or_default()
}

/// The remembered choice (ids of the people file; `None` = nobody).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Choice {
    pub presenter: Option<String>,
    pub supervisor: Option<String>,
}

fn choice_file(ws_root: &Path) -> PathBuf {
    ws_root.join("paper-deck.json")
}

pub fn last(ws_root: &Path) -> Choice {
    std::fs::read(choice_file(ws_root)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn remember(ws_root: &Path, c: &Choice) -> std::io::Result<()> {
    if last(ws_root) == *c {
        return Ok(());
    }
    crate::jobs::write_atomic(&choice_file(ws_root), &serde_json::to_vec_pretty(c).unwrap_or_default())
}

/// The names for a job: `asked` (ids from the request; `Some("")` = nobody) over the
/// remembered choice; an id the file no longer has is nobody. What was asked is remembered.
pub fn resolve(ws_root: &Path, people: &[Person], asked: (Option<&str>, Option<&str>)) -> (Option<String>, Option<String>) {
    let mut c = last(ws_root);
    let mut changed = false;
    for (slot, a) in [(&mut c.presenter, asked.0), (&mut c.supervisor, asked.1)] {
        if let Some(a) = a {
            let v = Some(a.trim().to_string()).filter(|x| !x.is_empty());
            changed |= *slot != v;
            *slot = v;
        }
    }
    if changed {
        let _ = remember(ws_root, &c);
    }
    let name = |id: &Option<String>| id.as_ref().and_then(|id| people.iter().find(|p| &p.id == id)).map(|p| p.name.clone());
    (name(&c.presenter), name(&c.supervisor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_are_an_id_and_a_name_and_the_choice_is_remembered() {
        let csv = "id,given,family,email,orcid,affiliations,role,note\nA001,Ellrin,Example,ellrin@example.org,0000-0000-0000-0001,F001,me,\nA002,Daidai,\"Example, Jr.\",daidai@example.org,,F001;F002,advisor,\"says \"\"hi\"\"\"\n,No,Id,,,,,\n";
        let p = parse(csv);
        assert_eq!(p, [Person { id: "A001".into(), name: "Ellrin Example".into() }, Person { id: "A002".into(), name: "Daidai Example, Jr.".into() }]);
        assert!(!serde_json::to_string(&p).unwrap().contains("example.org"), "no e-mail leaves the file");
        let d = tempfile::tempdir().unwrap();
        // nothing remembered, nothing asked: nobody
        assert_eq!(resolve(d.path(), &p, (None, None)), (None, None));
        assert_eq!(resolve(d.path(), &p, (Some("A001"), Some("A002"))), (Some("Ellrin Example".into()), Some("Daidai Example, Jr.".into())));
        // the next job without a choice (直接產生 from an older page) takes the remembered one
        assert_eq!(resolve(d.path(), &p, (None, None)).0.as_deref(), Some("Ellrin Example"));
        // "" = nobody, remembered too; an id that is gone is nobody
        assert_eq!(resolve(d.path(), &p, (None, Some(""))), (Some("Ellrin Example".into()), None));
        assert_eq!(last(d.path()), Choice { presenter: Some("A001".into()), supervisor: None });
        assert_eq!(resolve(d.path(), &p[1..], (None, None)), (None, None));
    }
}
