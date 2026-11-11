//! End-to-end tests over temp spaces (personal + household). Offline; no iCloud:
//! the personal space's asset root points into the temp dir.

use std::path::Path;
use std::sync::Arc;

use kb_core::index::Filter;
use kb_core::node::{Image, Link};
use kb_core::{FixedClock, GraphQuery, KbError, KbOptions, KnowledgeBase, NodeFields, Outcome, ScanOptions};
use lore_store::{AssetsConfig, Space, SpaceConfig, SpaceKind, Spaces};

struct Env {
    _dir: tempfile::TempDir,
    clock: Arc<FixedClock>,
    kb: KnowledgeBase,
    root: std::path::PathBuf,
}

fn personal(root: &Path) -> SpaceConfig {
    SpaceConfig {
        id: "me".into(),
        name: "我的知識庫".into(),
        kind: SpaceKind::Personal,
        members: vec!["ellrin".into()],
        assets: AssetsConfig { backend: Some(lore_store::space::BackendKind::Icloud), root: Some(root.join("icloud").display().to_string()), ..Default::default() },
    }
}

fn household() -> SpaceConfig {
    SpaceConfig { id: "home".into(), name: "家庭".into(), kind: SpaceKind::Household, members: vec!["ellrin".into(), "daidai".into()], assets: AssetsConfig::default() }
}

fn env_as(member: &str) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    Space::create(root.join("personal"), personal(&root)).unwrap();
    Space::create(root.join("household"), household()).unwrap();
    std::fs::write(root.join("personal/apps.toml"), "[apps.slidecomposer]\nlink_types = [{ id = \"illustrates\", label = \"圖解\" }]\n[apps.slidecomposer.kinds.figure]\nlabel = \"圖\"\n").unwrap();
    let clock = Arc::new(FixedClock::at("2026-11-08T21:10:00+08:00"));
    let kb = open(&root, member, clock.clone());
    Env { _dir: dir, clock, kb, root }
}

fn open(root: &Path, member: &str, clock: Arc<FixedClock>) -> KnowledgeBase {
    let mut spaces = Spaces::new();
    spaces.mount(root.join("personal")).unwrap();
    spaces.mount(root.join("household")).unwrap();
    KnowledgeBase::open(spaces, member, KbOptions { clock, ..Default::default() }).unwrap()
}

fn fields(title: &str, aliases: &[&str]) -> NodeFields {
    NodeFields { title: Some(title.into()), aliases: Some(aliases.iter().map(|s| s.to_string()).collect()), ..Default::default() }
}

fn png() -> Vec<u8> {
    // 1×1 PNG.
    vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1F, 0x15, 0xC4, 0x89, 0, 0, 0, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0, 0x05, 0, 0x01, 0xFF, 0x89, 0x99, 0x3D, 0x1D, 0,
        0, 0, 0, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ]
}

#[test]
fn create_edit_trash_and_files_are_truth() {
    let mut e = env_as("ellrin");
    let n = e.kb.create("me", NodeFields { tags: Some(vec!["deep-learning/cnn".into()]), domains: Some(vec!["computer-vision".into()]), ..fields("U-Net", &["UNet"]) }, Some("test")).unwrap();
    assert!(n.id.starts_with('n') && n.id.len() == 27);
    let file = e.root.join("personal/nodes").join(format!("{}.json", n.id));
    assert!(file.is_file());
    assert!(e.root.join("personal/progress/ellrin").join(format!("{}.json", n.id)).is_file());

    e.clock.advance_days(1);
    let u = e.kb.update(&n.id, NodeFields { summary: Some("編碼器—解碼器分割網路".into()), ..Default::default() }, Some(1)).unwrap();
    assert_eq!(u.revision, 2);
    assert_eq!(u.updated_at.to_rfc3339(), "2026-11-09T21:10:00+08:00");
    assert_eq!(u.created_at, n.created_at);
    assert!(matches!(e.kb.update(&n.id, NodeFields::default(), Some(1)), Err(KbError::Conflict { expected: 1, actual: 2 })));

    // Filters: tag hierarchy, domain, kind, state, FTS (CJK), dates.
    let q = |f: Filter| e.kb.list(&f).unwrap().into_iter().map(|s| s.title).collect::<Vec<_>>();
    assert_eq!(q(Filter { tag: Some("deep-learning".into()), ..Default::default() }), vec!["U-Net"]);
    assert!(q(Filter { tag: Some("deep".into()), ..Default::default() }).is_empty());
    assert_eq!(q(Filter { domain: Some("computer-vision".into()), ..Default::default() }), vec!["U-Net"]);
    assert_eq!(q(Filter { state: Some("learning".into()), ..Default::default() }), vec!["U-Net"]);
    assert_eq!(q(Filter { q: Some("解碼器".into()), ..Default::default() }), vec!["U-Net"]);
    assert_eq!(q(Filter { q: Some("碼".into()), ..Default::default() }), vec!["U-Net"]);
    assert_eq!(q(Filter { q: Some("unet".into()), ..Default::default() }), vec!["U-Net"]);
    assert!(q(Filter { q: Some("transformer".into()), ..Default::default() }).is_empty());
    assert_eq!(q(Filter { since: Some("2026-11-08".into()), until: Some("2026-11-08".into()), ..Default::default() }), vec!["U-Net"]);
    assert!(q(Filter { since: Some("2026-11-09".into()), ..Default::default() }).is_empty());
    assert_eq!(q(Filter { since: Some("2026-11-09".into()), date_field: Some("updated".into()), ..Default::default() }), vec!["U-Net"]);

    // The index is rebuildable from files.
    let kb2 = open(&e.root, "ellrin", e.clock.clone());
    assert_eq!(kb2.list(&Filter::default()).unwrap().len(), 1);
    assert_eq!(kb2.progress(&n.id).unwrap().unwrap().streak, 0);

    let to = e.kb.trash(&n.id).unwrap();
    assert!(to.starts_with(e.root.join("personal/trash/nodes")));
    assert!(!file.exists());
    assert!(e.root.join("personal/trash/progress/ellrin").join(format!("{}.json", n.id)).is_file());
    assert!(e.kb.list(&Filter::default()).unwrap().is_empty());
    assert!(matches!(e.kb.node(&n.id), Err(KbError::NotFound(_))));

    // Undo: the card and its learning state come back; a second restore finds nothing.
    let (sid, back) = e.kb.restore(&n.id).unwrap();
    assert_eq!((sid.as_str(), back.revision), ("me", 2));
    assert!(file.is_file());
    assert!(e.root.join("personal/progress/ellrin").join(format!("{}.json", n.id)).is_file());
    assert_eq!(e.kb.list(&Filter::default()).unwrap().len(), 1);
    assert!(e.kb.progress(&n.id).unwrap().is_some());
    assert!(matches!(e.kb.restore(&n.id), Err(KbError::Invalid(_))));
    // Trashed twice: the latest copy is restored.
    e.kb.trash(&n.id).unwrap();
    e.kb.restore(&n.id).unwrap();
    e.kb.update(&n.id, NodeFields { summary: Some("第三版".into()), ..Default::default() }, None).unwrap();
    e.kb.trash(&n.id).unwrap();
    assert_eq!(e.kb.restore(&n.id).unwrap().1.summary, "第三版");
    assert!(matches!(e.kb.restore("n01K7QHM527JNK6KJFYY0WPJ28S"), Err(KbError::NotFound(_))));
}

#[test]
fn two_remembered_make_known_via_due_queue() {
    let mut e = env_as("ellrin");
    let n = e.kb.create("me", fields("Dice coefficient", &["DSC"]), None).unwrap();
    assert!(e.kb.due(10).unwrap().is_empty());
    e.clock.advance_days(1);
    assert_eq!(e.kb.due(10).unwrap()[0].id, n.id);
    let r = e.kb.recall(&n.id, Outcome::Remembered, Some("desk")).unwrap();
    assert!(r.step.advanced && r.progress.streak == 1);
    assert!(e.kb.due(10).unwrap().is_empty());
    e.clock.advance_days(3);
    let r = e.kb.recall(&n.id, Outcome::Remembered, Some("desk")).unwrap();
    assert!(r.step.became_known);
    assert_eq!(e.kb.summary(&n.id).unwrap().unwrap().state.as_deref(), Some("known"));
    // Forgot resets.
    e.clock.advance_days(30);
    let r = e.kb.recall(&n.id, Outcome::Forgot, None).unwrap();
    assert_eq!((r.progress.state.as_str(), r.progress.streak), ("learning", 0));
    let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(e.root.join("personal/progress/ellrin").join(format!("{}.json", n.id))).unwrap()).unwrap();
    assert_eq!(saved["state"], "learning");
    assert!(saved["log"].as_array().unwrap().len() >= 5);
}

#[test]
fn create_as_known_skips_the_learning_checks() {
    let mut e = env_as("ellrin");
    let n = e.kb.create_as("me", fields("Softmax", &[]), Some("reader"), kb_core::State::Known).unwrap();
    assert_eq!(e.kb.summary(&n.id).unwrap().unwrap().state.as_deref(), Some("known"));
    let p = e.kb.progress(&n.id).unwrap().unwrap();
    assert_eq!((p.streak, p.maintenance_step), (0, 0));
    assert_eq!(p.log.iter().map(|l| l.event.as_str()).collect::<Vec<_>>(), ["created", "known"]);
    e.clock.advance_days(1);
    assert!(e.kb.due(10).unwrap().is_empty(), "no first check for a known card");
    e.clock.advance_days(29);
    assert_eq!(e.kb.due(10).unwrap()[0].id, n.id, "first maintenance check at +30 d");
    // The default is unchanged, and a new card cannot start stale.
    let l = e.kb.create("me", fields("Sigmoid", &[]), None).unwrap();
    assert_eq!(e.kb.summary(&l.id).unwrap().unwrap().state.as_deref(), Some("learning"));
    assert!(e.kb.create_as("me", fields("Tanh", &[]), None, kb_core::State::Stale).is_err());
}

#[test]
fn scan_finds_aliases_with_state_and_records_encounters() {
    let mut e = env_as("ellrin");
    let dice = e.kb.create("me", fields("Dice coefficient", &["DSC", "Dice score"]), None).unwrap();
    let knee = e.kb.create("me", fields("膝關節", &["knee joint"]), None).unwrap();
    let r = e.kb.scan("The DSC of knee joints; 膝關節軟骨與 dice scores.", &ScanOptions { record: true, candidates: true, via: Some("reader".into()), ..Default::default() }).unwrap();
    let got: Vec<(&str, &str, bool, &str)> = r.spans.iter().map(|s| (s.text.as_str(), s.node_id.as_str(), s.first, s.state.as_str())).collect();
    assert_eq!(
        got,
        vec![
            ("DSC", dice.id.as_str(), true, "learning"),
            ("knee joints", knee.id.as_str(), true, "learning"),
            ("膝關節", knee.id.as_str(), false, "learning"),
            ("dice scores", dice.id.as_str(), false, "learning"),
        ]
    );
    assert_eq!(e.kb.progress(&dice.id).unwrap().unwrap().encounters, 1);
    assert_eq!(r.assistant.as_deref(), Some("mock"));
    // The scanner follows edits.
    e.kb.update(&dice.id, NodeFields { aliases: Some(vec!["Sørensen–Dice".into()]), ..Default::default() }, None).unwrap();
    let r = e.kb.scan("SØRENSEN–DICE but not DSC", &ScanOptions::default()).unwrap();
    assert_eq!(r.spans.len(), 1);
}

#[test]
fn links_mentions_backlinks_graph_timeline() {
    let mut e = env_as("ellrin");
    let iou = e.kb.create("me", fields("IoU", &["Jaccard index"]), None).unwrap();
    let dice = e
        .kb
        .create("me", NodeFields { body: Some("和 [[jaccard index]] 很像，見 [[不存在]]".into()), links: Some(vec![Link { to: iou.id.clone(), rel: "contrast".into(), extra: Default::default() }]), tags: Some(vec!["metric".into()]), ..fields("Dice", &[]) }, None)
        .unwrap();
    let v = e.kb.view(&iou.id).unwrap();
    let rels: Vec<&str> = v.backlinks.iter().map(|b| b.rel.as_str()).collect();
    assert!(rels.contains(&"contrast") && rels.contains(&"mentions"));
    assert_eq!(e.kb.view(&dice.id).unwrap().unresolved, vec!["不存在"]);
    // Unknown link type / kind; app-declared ones work.
    assert!(e.kb.update(&dice.id, NodeFields { links: Some(vec![Link { to: iou.id.clone(), rel: "frobs".into(), extra: Default::default() }]), ..Default::default() }, None).is_err());
    e.kb.update(&dice.id, NodeFields { links: Some(vec![Link { to: iou.id.clone(), rel: "illustrates".into(), extra: Default::default() }]), ..Default::default() }, None).unwrap();
    assert!(e.kb.create("me", NodeFields { kind: Some("figure".into()), ..fields("架構圖", &[]) }, None).is_ok());
    assert!(e.kb.create("me", NodeFields { kind: Some("nope".into()), ..fields("x", &[]) }, None).is_err());

    let g = e.kb.graph(Some(&iou.id), 1, true, None, 100).unwrap();
    assert!(g.nodes.iter().any(|n| n.id == dice.id));
    assert!(g.nodes.iter().any(|n| n.id == "tag:metric"));
    assert_eq!(e.kb.graph(None, 2, false, None, 100).unwrap().nodes.len(), 3);

    e.clock.advance_days(2);
    e.kb.update(&iou.id, NodeFields { summary: Some("交集除以聯集".into()), ..Default::default() }, None).unwrap();
    let t = e.kb.timeline(7, None).unwrap();
    assert_eq!(t[0].day, "2026-11-10");
    assert_eq!(t[0].updated[0].id, iou.id);
    assert_eq!(t[1].day, "2026-11-08");
    // Dice was edited on the 8th too; IoU created then.
    assert_eq!(t[1].created.len(), 3);
    assert_eq!(e.kb.recent(1, 10).unwrap().len(), 1);
}

#[test]
fn graph_fields_placeholders_and_local_depth() {
    let mut e = env_as("ellrin");
    let link = |to: &str, rel: &str| Link { to: to.into(), rel: rel.into(), extra: Default::default() };
    let a = e.kb.create("me", NodeFields { summary: Some("卷積".into()), tags: Some(vec!["deep-learning/cnn".into()]), domains: Some(vec!["deep-learning".into()]), ..fields("Convolution", &["conv"]) }, None).unwrap();
    e.clock.advance_days(1);
    let b = e.kb.create("me", NodeFields { links: Some(vec![link(&a.id, "prerequisite")]), body: Some("見 [[Pooling]] 與 [[pooling]]、[[Stride]]".into()), ..fields("CNN", &[]) }, None).unwrap();
    e.clock.advance_days(1);
    let c = e.kb.create("me", NodeFields { links: Some(vec![link(&b.id, "example-of")]), ..fields("U-Net", &[]) }, None).unwrap();
    let lone = e.kb.create("me", fields("Orphan", &[]), None).unwrap();
    e.kb.create("home", fields("家用卡", &[]), None).unwrap();

    // Global, every field; placeholders only on request, one per unresolved name (case-folded).
    let g = e.kb.graph_with(&GraphQuery { missing: true, limit: 10_000, ..Default::default() }).unwrap();
    assert_eq!((g.total, g.truncated, g.center.as_deref()), (5, false, None));
    let n = |id: &str| g.nodes.iter().find(|n| n.id == id).unwrap().clone();
    let conv = n(&a.id);
    assert_eq!((conv.summary.as_str(), conv.aliases.clone(), conv.domains.clone(), conv.degree), ("卷積", vec!["conv".to_string()], vec!["deep-learning".to_string()], 1));
    assert_eq!(conv.state.as_deref(), Some("learning"));
    assert!(conv.created_at.starts_with("2026-11-08T21:10:00"));
    assert!(n(&c.id).created_at.starts_with("2026-11-10"));
    assert_eq!(n(&b.id).degree, 4, "prerequisite + example-of + two placeholders");
    assert_eq!((n(&lone.id).degree, n(&lone.id).hop), (0, None));
    let missing: Vec<&str> = g.nodes.iter().filter(|n| n.kind == "missing").map(|n| n.title.as_str()).collect();
    assert_eq!(missing, vec!["Pooling", "Stride"]);
    assert!(g.nodes.iter().any(|n| n.id == "missing:me:pooling" && n.space == "me" && n.degree == 1));
    assert!(g.edges.iter().any(|x| x.from == b.id && x.to == "missing:me:stride" && x.rel == "mentions"));
    assert_eq!(e.kb.graph_with(&GraphQuery { limit: 10_000, ..Default::default() }).unwrap().nodes.iter().filter(|n| n.kind == "missing").count(), 0);

    // Global limit keeps the newest cards and reports truncation.
    let g = e.kb.graph_with(&GraphQuery { limit: 2, space: Some("me".into()), ..Default::default() }).unwrap();
    assert_eq!((g.total, g.truncated, g.nodes.len()), (4, true, 2));

    // Local: hop distances, depth, placeholders hang off kept cards only.
    let g = e.kb.graph_with(&GraphQuery { center: Some(c.id.clone()), depth: 1, missing: true, limit: 100, ..Default::default() }).unwrap();
    let hops: Vec<(String, Option<u32>)> = g.nodes.iter().map(|n| (n.title.clone(), n.hop)).collect();
    assert_eq!(hops, vec![("U-Net".to_string(), Some(0)), ("CNN".to_string(), Some(1))]);
    assert_eq!(g.center.as_deref(), Some(c.id.as_str()));
    let g = e.kb.graph_with(&GraphQuery { center: Some(c.id.clone()), depth: 2, missing: true, limit: 100, ..Default::default() }).unwrap();
    assert_eq!(g.nodes.iter().filter(|n| n.hop == Some(2)).count(), 3, "Convolution + two placeholders");
    assert_eq!(g.total, 3);
    let g = e.kb.graph_with(&GraphQuery { center: Some(c.id.clone()), depth: 3, missing: true, limit: 2, ..Default::default() }).unwrap();
    assert_eq!((g.total, g.truncated), (3, true));
    assert!(g.nodes.iter().all(|n| n.kind != "missing" || g.edges.iter().any(|x| x.to == n.id)));
    // Unknown center → global graph.
    assert!(e.kb.graph_with(&GraphQuery { center: Some("nope".into()), ..Default::default() }).unwrap().center.is_none());
    // Placeholders disappear once the card exists.
    e.kb.create("me", fields("Stride", &[]), None).unwrap();
    let g = e.kb.graph_with(&GraphQuery { missing: true, limit: 100, ..Default::default() }).unwrap();
    assert_eq!(g.nodes.iter().filter(|n| n.kind == "missing").count(), 1);
}

#[test]
fn household_isolation_and_explicit_share() {
    let mut e = env_as("ellrin");
    // Personal card with an image and a link to another personal card.
    let img = e.kb.put_asset("me", &png(), "png").unwrap();
    assert!(e.root.join("icloud").join(img.rel_path()).is_file());
    let iou = e.kb.create("me", fields("IoU", &[]), None).unwrap();
    let dice = e
        .kb
        .create(
            "me",
            NodeFields {
                images: Some(vec![Image { asset: img.asset.clone(), ext: img.ext.clone(), caption: "Dice".into(), credit: String::new(), marks: vec![], extra: Default::default() }]),
                links: Some(vec![Link { to: iou.id.clone(), rel: "contrast".into(), extra: Default::default() }]),
                ..fields("Dice coefficient", &["DSC"])
            },
            None,
        )
        .unwrap();

    // A household card may not link to a personal card.
    let h = e.kb.create("home", fields("segmentation", &[]), None).unwrap();
    let err = e.kb.update(&h.id, NodeFields { links: Some(vec![Link { to: dice.id.clone(), rel: "related".into(), extra: Default::default() }]), ..Default::default() }, None).unwrap_err();
    assert!(matches!(err, KbError::Forbidden(_)));

    // Share: a copy with a new id, no personal ids, image re-encoded into the household repo.
    let copy = e.kb.share(&dice.id, "home").unwrap();
    assert_ne!(copy.id, dice.id);
    assert!(copy.links.is_empty(), "link to a personal-only card is dropped");
    assert!(copy.copies.is_empty());
    assert_eq!(copy.images[0].ext, "webp");
    let hfile = std::fs::read_to_string(e.root.join("household/nodes").join(format!("{}.json", copy.id))).unwrap();
    assert!(!hfile.contains(&dice.id) && !hfile.contains(&iou.id));
    assert!(e.root.join("household/assets").join(lore_store::AssetRef::parse(&copy.images[0].asset, "webp").unwrap().rel_path()).is_file());
    // The personal side remembers the copy.
    let (_, src) = e.kb.node(&dice.id).unwrap();
    assert_eq!(src.copies[0].id, copy.id);
    assert_eq!(src.copies[0].dir, "to");

    // Share IoU too; sharing Dice again now keeps the link (remapped to IoU's copy).
    let iou_copy = e.kb.share(&iou.id, "home").unwrap();
    let copy2 = e.kb.share(&dice.id, "home").unwrap();
    assert_eq!(copy2.links[0].to, iou_copy.id);

    // Nothing in the household folder names a personal id.
    let personal_ids: Vec<String> = e.kb.list(&Filter { space: Some("me".into()), ..Default::default() }).unwrap().into_iter().map(|s| s.id).collect();
    for f in std::fs::read_dir(e.root.join("household/nodes")).unwrap() {
        let text = std::fs::read_to_string(f.unwrap().path()).unwrap();
        for id in &personal_ids {
            assert!(!text.contains(id.as_str()));
        }
    }

    // Household → personal: the personal copy records where it came from.
    let back = e.kb.share(&h.id, "me").unwrap();
    assert_eq!(back.copies[0].space, "home");
    assert_eq!(back.copies[0].dir, "from");

    // Learning state is per member: daidai sees household cards without ellrin's state
    // and cannot see or touch the personal space's progress.
    let mut spaces = Spaces::new();
    spaces.mount(e.root.join("household")).unwrap();
    let mut dd = KnowledgeBase::open(spaces, "daidai", KbOptions { clock: e.clock.clone(), ..Default::default() }).unwrap();
    let s = dd.summary(&copy.id).unwrap().unwrap();
    assert_eq!(s.state, None);
    dd.recall(&copy.id, Outcome::Unfamiliar, None).unwrap();
    assert!(e.root.join("household/progress/daidai").join(format!("{}.json", copy.id)).is_file());
    assert_eq!(e.kb.summary(&copy.id).unwrap().unwrap().state.as_deref(), Some("learning"));
    // daidai is not a member of the personal space.
    let mut both = Spaces::new();
    both.mount(e.root.join("personal")).unwrap();
    let dd2 = KnowledgeBase::open(both, "daidai", KbOptions { clock: e.clock.clone(), ..Default::default() }).unwrap();
    assert!(dd2.progress(&dice.id).unwrap().is_none());
}

#[test]
fn draft_through_mock_assistant() {
    let e = env_as("ellrin");
    let d = e.kb.draft(&kb_core::DraftRequest { term: "FID".into(), context: "We report FID on 5k samples. Lower is better.".into(), ..Default::default() }).unwrap();
    assert_eq!(d.title.as_deref(), Some("FID"));
    assert_eq!(d.summary.as_deref(), Some("We report FID on 5k samples."));
}

mod pending_tests {
    use super::*;
    use kb_core::NewPending;

    fn np(title: &str) -> NewPending {
        NewPending { title: title.into(), summary: "s".into(), body: "b".into(), source: "chat".into(), from_conversation: Some("c1".into()), ..Default::default() }
    }

    #[test]
    fn storage_and_exclusion() {
        let mut e = env_as("ellrin");
        let kb = &mut e.kb;
        let made = kb.pending_add("me", vec![np("Softmax"), np("Sigmoid")]).unwrap();
        assert!(e.root.join("personal/pending").join(format!("{}.json", made[0].id)).is_file());
        assert!(kb.pending_add("me", vec![np("ok"), np("  ")]).is_err());
        assert_eq!(kb.pending_list(None).unwrap().len(), 2);
        assert_eq!(kb.pending_list(Some("home")).unwrap().len(), 0);
        assert!(kb.list(&Filter::default()).unwrap().is_empty());
        assert!(kb.list(&Filter { q: Some("Softmax".into()), ..Default::default() }).unwrap().is_empty());
        assert!(kb.due(50).unwrap().is_empty());
        kb.rebuild().unwrap();
        assert!(kb.list(&Filter::default()).unwrap().is_empty());
        assert_eq!(kb.pending_get("me", &made[1].id).unwrap().title, "Sigmoid");
        assert!(kb.pending_get("me", "../x").is_err());
    }

    #[test]
    fn accept_discard_merge() {
        let mut e = env_as("ellrin");
        let kb = &mut e.kb;
        let old = kb.create("me", fields("Softmax", &["SM"]), None).unwrap();
        let mut a = np("softmax");
        a.links = vec![Link { to: old.id.clone(), rel: "bogus".into(), extra: Default::default() }, Link { to: "nXXXXXXXXXXXXXXXXXXXXXXXXXX".into(), rel: "related".into(), extra: Default::default() }];
        a.kind = "nokind".into();
        let ids = kb.pending_add("me", vec![a, np("Sigmoid"), np("Tanh")]).unwrap();
        let dups = kb.pending_duplicates("me", &ids[0]).unwrap();
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0].id, old.id);
        assert!(kb.pending_duplicates("me", &ids[1]).unwrap().is_empty());

        let n = kb.pending_accept("me", &ids[0].id, Some(NodeFields { title: Some("Softmax 2".into()), ..Default::default() })).unwrap();
        assert_eq!((n.title.as_str(), n.kind.as_str(), n.links.len(), n.links[0].rel.as_str()), ("Softmax 2", "concept", 1, "related"));
        assert_eq!(n.sources[0].reference, "chat");
        assert_eq!(kb.progress(&n.id).unwrap().unwrap().state, kb_core::State::Learning);
        assert!(kb.pending_get("me", &ids[0].id).is_err());
        assert!(e.root.join("personal/trash/pending").join(format!("{}.json", ids[0].id)).is_file());

        kb.pending_discard("me", &ids[1].id).unwrap();
        assert!(kb.pending_discard("me", &ids[1].id).is_err());

        let m = kb.pending_merge("me", &ids[2].id, &old.id, None).unwrap();
        assert!(m.aliases.contains(&"Tanh".to_string()) && m.aliases.contains(&"SM".to_string()));
        assert_eq!((m.revision, m.summary.as_str(), m.body.as_str()), (2, "s", "b"));
        assert!(kb.pending_list(None).unwrap().is_empty());
        assert_eq!(kb.list(&Filter::default()).unwrap().len(), 2);
    }
}
