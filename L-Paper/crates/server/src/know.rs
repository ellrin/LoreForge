//! 知識中樞 (docs/brain-hub.md): the one place where what outside applications aligned to L-Paper's
//! writing categories is read and merged into the loaded profile (`pc_core::aligned`). Called from
//! `load_from` in lib.rs, which every load and reload of the profile goes through.
//!
//! Only the four categories below are ever asked for. An outside application's independent set
//! (`x.<source>.<name>`) is never asked for and is dropped if a reader returns it anyway. `pc.rules`
//! is not among them on purpose: an outside application must not change what L-Paper's checks enforce.

use std::path::Path;

use lore_know::{split_id, Item};
use pc_core::aligned::{Aligned, MergeReport, Row};
use pc_core::Profile;
use serde_json::Value;

/// The categories L-Paper declares for the writing data it merges (`know.toml` ids).
pub const PATTERNS: &str = "pc.patterns";
pub const EXPRESSIONS: &str = "pc.expressions";
pub const WORDING: &str = "pc.wording";
pub const EXEMPLARS: &str = "pc.exemplars";

/// Reads the items aligned to one category from a hub folder (`lore_know::reader::aligned`);
/// a function pointer so tests hand in synthetic items.
pub type AlignedReader = fn(&Path, &str) -> Vec<Item>;

fn text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// An item as plain columns. A column the format lets fall back to the item's title (template,
/// text, avoid: the hub's own `prompt()` does the same) does so; nothing else is filled in.
fn row_of(it: &Item) -> Row {
    let mut fields: std::collections::BTreeMap<String, String> = it.fields.iter().map(|(k, v)| (k.clone(), text(v))).collect();
    let title_col = match it.format.as_str() {
        lore_know::format::PATTERN => "template",
        lore_know::format::EXPRESSION => "text",
        lore_know::format::WORDING => "avoid",
        _ => "",
    };
    if !title_col.is_empty() && fields.get(title_col).is_none_or(|v| v.trim().is_empty()) && !it.title.trim().is_empty() {
        fields.insert(title_col.into(), it.title.clone());
    }
    Row { source: if it.source.name.trim().is_empty() { "hub".into() } else { it.source.name.clone() }, local: split_id(&it.id).map(|(_, l)| l.to_string()).unwrap_or_default(), fields }
}

/// What the hub holds for the four categories. Items that are not of the category asked for are
/// dropped (a reader cannot smuggle an independent set in).
pub fn read_aligned(hub: &Path, read: AlignedReader) -> Aligned {
    let of = |cat: &str| -> Vec<Row> { read(hub, cat).iter().filter(|i| i.category == cat).map(row_of).collect() };
    Aligned { patterns: of(PATTERNS), expressions: of(EXPRESSIONS), wording: of(WORDING), exemplars: of(EXEMPLARS) }
}

/// The merge point: no hub folder, an empty hub or nothing read = `profile` exactly as it was
/// (never an error). `hub` is `lore_know::reader::hub_dir`.
pub fn merge_hub(profile: &mut Profile, hub: Option<&Path>, read: AlignedReader) -> MergeReport {
    let Some(hub) = hub.filter(|h| h.is_dir()) else { return MergeReport::default() };
    pc_core::aligned::merge(profile, read_aligned(hub, read))
}

#[cfg(test)]
mod tests {
    use super::*;
    use lore_know::{item_id, Source};
    use pc_core::compose::{self, ComposeRequest};
    use pc_core::profile::{Manifest, Phrase, VocabEntry};
    use std::sync::Mutex;

    fn item(cat: &str, fmt: &str, local: &str, src: &str, f: &[(&str, &str)]) -> Item {
        let mut it = Item::new(cat, fmt, "", Source::external(src));
        it.id = item_id(cat, local);
        for (k, v) in f {
            it.set(k, *v);
        }
        it
    }

    fn pattern_item(cat: &str, local: &str, src: &str, template: &str) -> Item {
        item(cat, "pattern", local, src, &[("section", "Result"), ("function", "finding"), ("position", "opening"), ("template", template)])
    }

    /// The categories the merge asked the reader for, in order.
    static ASKED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// A synthetic hub: one aligned phrase, one aligned wording, and (whatever the category asked
    /// for) an item of an independent set offered along.
    fn fake(_hub: &Path, category: &str) -> Vec<Item> {
        ASKED.lock().unwrap().push(category.to_string());
        let mut v = vec![item("x.wordbox.words", "expression", "i1", "wordbox", &[("text", "INDEPENDENT-WORD")]), pattern_item("x.wordbox.words", "i2", "wordbox", "INDEPENDENT {x}.")];
        match category {
            PATTERNS => v.push(pattern_item(PATTERNS, "7", "wordbox", "Outside frame: {method} reaches {value}.")),
            WORDING => v.push(item(WORDING, "wording", "w", "wordbox", &[("avoid", "very"), ("prefer", "markedly")])),
            _ => {}
        }
        v
    }

    fn nothing(_hub: &Path, _category: &str) -> Vec<Item> {
        vec![]
    }

    /// A profile folder with the author's rows.
    fn author_dir(root: &Path) -> std::path::PathBuf {
        let dir = root.join("ws/profile");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("profile.toml"), "name = \"t\"\nschema_version = 1\n").unwrap();
        std::fs::write(dir.join("phrases.csv"), "id,section,function,position,template,example,tags,priority\nP001,Result,finding,opening,Author frame: {x} rises.,,,4\n").unwrap();
        std::fs::write(dir.join("vocabulary.csv"), "avoid,prefer,note\nutilize,use,\n").unwrap();
        std::fs::write(dir.join("rules.csv"), "id,type,scope,check,threshold,severity,message\n").unwrap();
        dir
    }

    fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
        let mut v: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| (e.file_name().to_string_lossy().to_string(), std::fs::read(e.path()).unwrap())).collect();
        v.sort();
        v
    }

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("pc-know-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_profile_carries_aligned_rows_when_the_hub_has_some() {
        let root = tmp("some");
        let dir = author_dir(&root);
        let hub = root.join("hub");
        std::fs::create_dir_all(&hub).unwrap();
        let l = crate::load_from(dir.clone(), Some(root.join("ws")), false, Some(&hub), fake);
        let p = l.profile.unwrap();
        let ids: Vec<&str> = p.phrases.iter().map(|x| x.id.as_str()).collect();
        assert_eq!(ids, ["P001", "x:wordbox:7"]);
        assert_eq!(p.phrases[1].priority, 4, "the lowest priority present");
        assert_eq!(p.vocabulary.iter().map(|v| v.avoid.as_str()).collect::<Vec<_>>(), ["utilize", "very"]);
        assert!(p.rules.is_empty(), "rules are never merged");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_hub_is_the_profile_as_before() {
        let root = tmp("none");
        let dir = author_dir(&root);
        let plain = serde_json::to_string(&Profile::load(&dir).unwrap().0).unwrap();
        // no hub folder at all: the reader is never called
        let missing = root.join("no-hub");
        let a = serde_json::to_string(&crate::load_from(dir.clone(), None, false, Some(&missing), fake).profile.unwrap()).unwrap();
        let b = serde_json::to_string(&crate::load_from(dir.clone(), None, false, None, fake).profile.unwrap()).unwrap();
        assert_eq!((&a, &b), (&plain, &plain));
        // an empty hub folder
        let hub = root.join("hub");
        std::fs::create_dir_all(&hub).unwrap();
        let c = serde_json::to_string(&crate::load_from(dir.clone(), None, false, Some(&hub), nothing).profile.unwrap()).unwrap();
        assert_eq!(c, plain);
        // the demo workspace is never merged
        let d = serde_json::to_string(&crate::load_from(dir, None, true, Some(&hub), fake).profile.unwrap()).unwrap();
        assert_eq!(d, plain);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn outside_rows_are_never_written_to_the_csv() {
        let root = tmp("csv");
        let dir = author_dir(&root);
        let hub = root.join("hub");
        std::fs::create_dir_all(&hub).unwrap();
        let before = files(&dir);
        let l = crate::load_from(dir.clone(), None, false, Some(&hub), fake);
        assert_eq!(files(&dir), before, "loading writes nothing");
        // the editor posts back every row it was given plus an outside one: refused, file untouched
        let mut t = pc_core::table::read_table(&dir, "phrases.csv").unwrap();
        assert_eq!(t.rows.len(), 1, "the editor shows the author's file only");
        let mut row = vec![String::new(); t.headers.len()];
        row[0] = l.profile.unwrap().phrases[1].id.clone();
        row[4] = "Outside frame: {method} reaches {value}.".into();
        t.rows.push(row);
        assert!(pc_core::table::write_table(&dir, "phrases.csv", &t).is_err());
        assert_eq!(files(&dir), before);
        // editing the outside row's text in place is refused too; the author's row still saves
        let mut t = pc_core::table::read_table(&dir, "phrases.csv").unwrap();
        t.rows[0][4] = "Author frame: {x} grows.".into();
        pc_core::table::write_table(&dir, "phrases.csv", &t).unwrap();
        assert!(std::fs::read_to_string(dir.join("phrases.csv")).unwrap().contains("grows"));
        // the derived 我的風格 never counts an outside row as his voice
        let pr = Profile::load(&dir).unwrap().0;
        let mut merged = pr.phrases.clone();
        merged.push(Phrase { id: "x:wordbox:7".into(), section: "Result".into(), function: "finding".into(), position: "opening".into(), template: "Outside.".into(), example: String::new(), tags: vec![], priority: 3 });
        let src = pc_core::styleprofile::own(&merged, &[], &[], &[]);
        assert_eq!(src.phrases.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    fn profile_with(phrase: &str) -> Profile {
        Profile {
            root: Default::default(),
            manifest: Manifest { name: "t".into(), schema_version: 1, language: "en".into() },
            phrases: vec![Phrase { id: "P1".into(), section: "Result".into(), function: "finding".into(), position: "opening".into(), template: phrase.into(), example: String::new(), tags: vec![], priority: 3 }],
            expressions: vec![],
            vocabulary: vec![VocabEntry { avoid: "utilize".into(), prefer: "use".into(), note: String::new(), source: String::new() }],
            rules: vec![],
            outside_exemplars: vec![],
            authors: vec![],
            affiliations: vec![],
            variants: Vec::new(),
            style_checks: Default::default(),
        }
    }

    fn request() -> ComposeRequest {
        ComposeRequest { points: "- recall improves by 12%".into(), section: "Result".into(), function: None, min_words: 10, max_words: 60, language: "en".into(), before: None, after: None, key_value: None, index: None, count: None, turn: 0 }
    }

    #[test]
    fn an_aligned_phrase_reaches_the_drafting_prompt() {
        let hub = tmp("prompt");
        let mut p = profile_with("Author frame: {x} rises.");
        let rep = merge_hub(&mut p, Some(&hub), fake);
        assert_eq!(rep.sources, ["wordbox"]);
        let (req, pts) = (request(), compose::parse_points(&request().points));
        let refs = compose::reference_phrases(&p, &req, &pts);
        let prompt = compose::build_prompt(&p, &req, &pts, &refs, "");
        assert!(prompt.user.contains("Author frame: {x} rises."), "{}", prompt.user);
        assert!(prompt.user.contains("Outside frame: {method} reaches {value}."), "{}", prompt.user);
        assert!(prompt.user.contains("x:wordbox:7"), "the model can name the phrase it used");
        // outside wording is offered to the model but never enforced by the draft's own check
        assert!(prompt.system.contains("\"very\""));
        let mut d = compose::Draft { sentences: vec![compose::Sentence { text: "It is very good.".into(), phrase_id: None }] };
        assert!(compose::check_draft(&p, &req, &pts, &refs, &mut d).problems.iter().all(|m| !m.contains("very")));
        let _ = std::fs::remove_dir_all(&hub);
    }

    #[test]
    fn an_independent_set_never_reaches_a_prompt() {
        let hub = tmp("indep");
        ASKED.lock().unwrap().clear();
        let mut p = profile_with("Author frame: {x} rises.");
        merge_hub(&mut p, Some(&hub), fake);
        let asked = ASKED.lock().unwrap().clone();
        assert!(!asked.is_empty() && asked.iter().all(|c| c.starts_with("pc.")), "{asked:?}");
        assert!(!asked.iter().any(|c| c.starts_with("x.")));
        assert!(!asked.iter().any(|c| c == "pc.rules"), "rules are not merged");
        // a reader that returns the independent set anyway: dropped
        let (req, pts) = (request(), compose::parse_points(&request().points));
        let refs = compose::reference_phrases(&p, &req, &pts);
        let prompt = compose::build_prompt(&p, &req, &pts, &refs, "");
        for t in [&prompt.system, &prompt.user] {
            assert!(!t.contains("INDEPENDENT"), "{t}");
        }
        assert!(p.expressions.is_empty() && p.phrases.iter().all(|x| !x.template.contains("INDEPENDENT")));
        let _ = std::fs::remove_dir_all(&hub);
    }

    #[test]
    fn an_aligned_exemplar_reaches_the_voice_block_named_apart() {
        let root = tmp("voice");
        let dir = author_dir(&root);
        let (ws, hub) = (root.join("ws"), root.join("hub"));
        std::fs::create_dir_all(&hub).unwrap();
        std::fs::write(dir.join("exemplars.csv"), "id,section,function,text,source\nE1,Result,finding,\"Accuracy rose. These results indicate that the loss helps.\",\"Author (2024)\"\n").unwrap();
        std::fs::write(dir.join("phrases.csv"), "id,section,function,position,template,example,tags,priority\nP001,Result,finding,opening,Author frame: {x} rises.,,paper1,4\nP002,Result,finding,closing,These results indicate that {claim}.,,paper1,3\n").unwrap();
        fn exemplars(_h: &Path, c: &str) -> Vec<Item> {
            if c == EXEMPLARS { vec![item(EXEMPLARS, "exemplar", "o1", "wordbox", &[("section", "Result"), ("function", "finding"), ("text", "OUTSIDE paragraph about recall.")])] } else { vec![] }
        }
        let l = crate::load_from(dir.clone(), Some(ws.clone()), false, Some(&hub), exemplars);
        let prof = l.profile.unwrap();
        let passage = lore_polish::PresetPassage { text: "Recall rose on long inputs.", prefix: "", suffix: "", scope: "Result" };
        let with = crate::polish::preset_block_for(Some(&ws), Some(&prof), crate::polish::VOICE, &passage, &root.join("cache"));
        assert!(with.contains("Accuracy rose.") && with.contains("Paragraphs from outside sources") && with.contains("OUTSIDE paragraph about recall."), "{with}");
        let own_part = with.split("Paragraphs from outside sources").next().unwrap();
        assert!(!own_part.contains("OUTSIDE"), "his own paragraphs stay his: {own_part}");
        // without outside rows the block is byte for byte what it was
        let plain = Profile::load(&dir).unwrap().0;
        let without = crate::polish::preset_block_for(Some(&ws), Some(&plain), crate::polish::VOICE, &passage, &root.join("cache"));
        assert_eq!(with.split("\nParagraphs from outside sources").next().unwrap(), without.trim_end_matches('\n').to_string() + "\n".repeat(without.ends_with('\n') as usize).as_str());
        let _ = std::fs::remove_dir_all(&root);
    }
}
