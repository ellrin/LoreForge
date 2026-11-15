//! Decks bound to a .pptx where it lives: write-back, backup, external changes,
//! editor state in the file. Temp dirs and the synthetic deck only.

#[path = "../../../../commons/crates/lore-ooxml/tests/common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lore_canvas::Op;
use sc_core::{Disk, Workspace, Wrote};
use serde_json::json;

const STYLE: &str = r#"
[pack]
id = "test"
name = "Test"
slide_cx = 9144000
slide_cy = 5143500
[tags.title]
font = "Calibri"
size = 24.0
bold = true
[tags.body]
font = "Calibri"
size = 14.0
[palette.colors]
black = "000000"
"#;

struct Fx {
    _tmp: tempfile::TempDir,
    ws: Workspace,
    docs: PathBuf,
    file: PathBuf,
}

fn setup() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("private");
    std::fs::create_dir_all(root.join("styles/test")).unwrap();
    std::fs::write(root.join("styles/test/style.toml"), STYLE).unwrap();
    let docs = tmp.path().join("Documents ellrin");
    std::fs::create_dir_all(&docs).unwrap();
    let file = docs.join("talk.pptx");
    std::fs::write(&file, common::synthetic_deck()).unwrap();
    let ws = Workspace::new(&root, None);
    Fx { _tmp: tmp, ws, docs: docs.canonicalize().unwrap(), file: sc_core::file::canonical(&file) }
}

fn op(v: serde_json::Value) -> Op {
    serde_json::from_value(v).unwrap()
}

fn none() -> HashMap<String, Vec<u8>> {
    HashMap::new()
}

/// Move the picture (id 5) on slide 2 by `dx`.
fn move_picture(sess: &mut sc_core::Session, dx: i64) {
    let uid = sess.deck().slides[1].uid.clone();
    let mut f = sess.deck().slides[1].find(5).unwrap().frame.clone();
    f.x += dx;
    sess.editor.apply(vec![op(json!({"op": "set_obj", "slide": uid, "id": 5, "path": "/frame", "value": f}))], "移動").unwrap();
    sess.save().unwrap();
}

fn hidden_backups(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.file_name().unwrap().to_string_lossy().contains(".original")).collect();
    v.sort();
    v
}

#[test]
fn bound_deck_writes_back_losslessly() {
    let fx = setup();
    let original = std::fs::read(&fx.file).unwrap();
    let name = fx.ws.open_path(&fx.file).unwrap();
    assert_eq!(fx.ws.open_path(&fx.file).unwrap(), name, "same path, same deck");
    let mut sess = fx.ws.open(&name).unwrap();
    let st = sess.file_status().unwrap();
    assert_eq!((st.state, st.file.as_str()), ("saved", "talk.pptx"));
    // nothing changed: nothing written
    assert_eq!(sess.write_back(&none(), false).unwrap(), Wrote::Unchanged);
    assert_eq!(std::fs::read(&fx.file).unwrap(), original);

    move_picture(&mut sess, 100_000);
    assert_eq!(sess.file_status().unwrap().state, "pending");
    assert_eq!(sess.write_back(&none(), false).unwrap(), Wrote::Written);
    assert_eq!(sess.file_status().unwrap().state, "saved");

    let orig = lore_ooxml::Deck::from_bytes(&original).unwrap();
    let back = lore_ooxml::Deck::from_bytes(&std::fs::read(&fx.file).unwrap()).unwrap();
    // the untouched slide keeps its original bytes, the edited one does not
    let s1 = &orig.slides[0].part;
    assert_eq!(back.pkg.get(s1), orig.pkg.get(s1), "{s1}");
    assert_ne!(back.pkg.get(&orig.slides[1].part), orig.pkg.get(&orig.slides[1].part));
    // every other original part is still there unchanged, except the package bookkeeping
    for p in &orig.pkg.parts {
        if p.name == orig.slides[1].part || p.name.ends_with(".rels") || p.name == "[Content_Types].xml" {
            continue;
        }
        assert!(back.pkg.get(&p.name) == Some(p.data.as_slice()), "{}", p.name);
    }
    // the editor state rides along in a custom XML part
    let (part, state) = sc_core::file::read_state(&back.pkg, &back.pres_part).unwrap();
    assert!(part.starts_with("customXml/item"), "{part}");
    assert_eq!(state.deck_id, sess.file.as_ref().unwrap().deck_id);
    assert!(back.pkg.names().any(|n| n.starts_with("customXml/itemProps")));
    // no temp file left next to the document
    assert!(!std::fs::read_dir(&fx.docs).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".sc-tmp")));

    // a second write keeps one state part
    move_picture(&mut sess, 50_000);
    assert_eq!(sess.write_back(&none(), false).unwrap(), Wrote::Written);
    let back = lore_ooxml::Deck::from_bytes(&std::fs::read(&fx.file).unwrap()).unwrap();
    assert_eq!(back.pkg.names().filter(|n| n.starts_with("customXml/item") && !n.contains("Props") && !n.contains("_rels")).count(), 1);
    // the written file still opens as the same deck
    assert_eq!(fx.ws.open_path(&fx.file).unwrap(), name);
}

#[test]
fn original_is_backed_up_exactly_once() {
    let fx = setup();
    let original = std::fs::read(&fx.file).unwrap();
    let name = fx.ws.open_path(&fx.file).unwrap();
    let mut sess = fx.ws.open(&name).unwrap();
    assert!(hidden_backups(&fx.docs).is_empty(), "no backup before the first overwrite");
    for dx in [10_000, 20_000, 30_000] {
        move_picture(&mut sess, dx);
        assert_eq!(sess.write_back(&none(), false).unwrap(), Wrote::Written);
    }
    let b = hidden_backups(&fx.docs);
    assert_eq!(b, vec![fx.docs.join(".talk.original.pptx").canonicalize().unwrap()]);
    assert_eq!(std::fs::read(&b[0]).unwrap(), original);
    assert_eq!(sess.file_status().unwrap().backup.as_deref(), Some(b[0].as_path()));

    // deck folder wiped: reopening and saving again makes no second backup (the file says it is ours)
    drop(sess);
    std::fs::remove_dir_all(&fx.ws.decks).unwrap();
    let name = fx.ws.open_path(&fx.file).unwrap();
    let mut sess = fx.ws.open(&name).unwrap();
    move_picture(&mut sess, 40_000);
    assert_eq!(sess.write_back(&none(), false).unwrap(), Wrote::Written);
    assert_eq!(hidden_backups(&fx.docs).len(), 1);
    assert_eq!(std::fs::read(&b[0]).unwrap(), original, "the backup is never touched");
}

#[test]
fn a_file_changed_elsewhere_is_never_overwritten_silently() {
    let fx = setup();
    let name = fx.ws.open_path(&fx.file).unwrap();
    let mut sess = fx.ws.open(&name).unwrap();
    move_picture(&mut sess, 100_000);
    sess.write_back(&none(), false).unwrap();

    // someone else saves the file (here: back to the original synthetic deck)
    std::fs::write(&fx.file, common::synthetic_deck()).unwrap();
    assert_eq!(sess.file.as_ref().unwrap().disk(), Disk::Changed);
    assert_eq!(sess.file_status().unwrap().state, "changed");

    move_picture(&mut sess, 1);
    assert_eq!(sess.write_back(&none(), false).unwrap(), Wrote::Conflict(Disk::Changed));
    assert_eq!(std::fs::read(&fx.file).unwrap(), common::synthetic_deck(), "their version stays");

    // 重新載入: the deck becomes the file on disk
    drop(sess);
    fx.ws.reload_file(&name).unwrap();
    let mut sess = fx.ws.open(&name).unwrap();
    assert_eq!(sess.file_status().unwrap().state, "saved");
    let x0 = lore_ooxml::Deck::from_bytes(&common::synthetic_deck()).unwrap();
    let pic = |d: &lore_canvas::Deck| d.slides[1].find(5).unwrap().frame.x;
    assert_eq!(pic(sess.deck()), pic(&lore_canvas::pptx_in::import(&x0, None, None)), "their picture position");
    // 保留我的版本，覆蓋: forced write over a changed file
    move_picture(&mut sess, 5);
    std::fs::write(&fx.file, b"not a deck any more").unwrap();
    assert_eq!(sess.write_back(&none(), false).unwrap(), Wrote::Conflict(Disk::Changed));
    assert_eq!(sess.write_back(&none(), true).unwrap(), Wrote::Written);
    assert!(lore_ooxml::Deck::from_bytes(&std::fs::read(&fx.file).unwrap()).is_ok());

    // a vanished file is a conflict too
    std::fs::remove_file(&fx.file).unwrap();
    move_picture(&mut sess, 5);
    assert_eq!(sess.write_back(&none(), false).unwrap(), Wrote::Conflict(Disk::Missing));
    assert_eq!(sess.file_status().unwrap().state, "missing");
}

#[test]
fn editor_state_survives_a_wiped_deck_folder() {
    let fx = setup();
    let name = fx.ws.open_path(&fx.file).unwrap();
    let mut sess = fx.ws.open(&name).unwrap();
    let uid = sess.deck().slides[1].uid.clone();
    let empty = json!({ "id": 40, "name": "text-2", "slot": "text-2", "frame": { "x": 4572000, "y": 1000000, "w": 3000000, "h": 2000000 },
        "type": "text", "geom": { "prst": "rect" }, "fill": { "kind": "none" }, "text": { "paras": [ { "runs": [] } ] } });
    sess.editor.deck.style = Some("test".into());
    sess.editor
        .apply(
            vec![
                op(json!({"op": "set_slide", "slide": uid, "path": "/arrangement", "value": "halves-v:image,text"})),
                op(json!({"op": "set_obj", "slide": uid, "id": 5, "path": "/slot", "value": "image-1"})),
                op(json!({"op": "insert_obj", "slide": uid, "index": 1, "obj": empty})),
                op(json!({"op": "set_deck", "path": "/defaults/content", "value": "halves-v:image,text"})),
            ],
            "版型",
        )
        .unwrap();
    sess.save().unwrap();
    assert_eq!(sess.write_back(&none(), false).unwrap(), Wrote::Written);
    // the empty slot is not in the file's slide
    let back = lore_ooxml::Deck::from_bytes(&std::fs::read(&fx.file).unwrap()).unwrap();
    assert!(!String::from_utf8_lossy(back.pkg.get(&back.slides[1].part).unwrap()).contains("text-2"));
    let before = sess.deck().clone();
    drop(sess);

    // cache wiped
    std::fs::remove_dir_all(&fx.ws.decks).unwrap();
    let again = fx.ws.open_path(&fx.file).unwrap();
    let sess = fx.ws.open(&again).unwrap();
    let d = sess.deck();
    assert_eq!(d.style.as_deref(), Some("test"));
    assert_eq!(d.defaults.get("content").map(String::as_str), Some("halves-v:image,text"));
    let s2 = &d.slides[1];
    assert_eq!(s2.arrangement.as_deref(), Some("halves-v:image,text"));
    assert_eq!(s2.find(5).unwrap().slot.as_deref(), Some("image-1"));
    let e = s2.objects.iter().find(|o| o.slot.as_deref() == Some("text-2")).expect("empty slot restored");
    assert!(e.is_empty_slot());
    assert_eq!(s2.objects.iter().position(|o| o.slot.as_deref() == Some("text-2")), before.slides[1].objects.iter().position(|o| o.slot.as_deref() == Some("text-2")));
    assert_eq!(s2.objects.len(), before.slides[1].objects.len());
    // bound to the same file, nothing pending, and the next save makes no new backup
    let st = sess.file_status().unwrap();
    assert_eq!((st.path.as_path(), st.state), (fx.file.as_path(), "saved"));
}

#[cfg(unix)]
#[test]
fn a_failed_write_leaves_the_file_intact() {
    use std::os::unix::fs::PermissionsExt;
    let fx = setup();
    let name = fx.ws.open_path(&fx.file).unwrap();
    let mut sess = fx.ws.open(&name).unwrap();
    move_picture(&mut sess, 100_000);
    sess.write_back(&none(), false).unwrap();
    let written = std::fs::read(&fx.file).unwrap();

    // the folder becomes read-only: no temp file can be made, the rename never happens
    std::fs::set_permissions(&fx.docs, std::fs::Permissions::from_mode(0o555)).unwrap();
    move_picture(&mut sess, 100_000);
    let r = sess.write_back(&none(), false);
    std::fs::set_permissions(&fx.docs, std::fs::Permissions::from_mode(0o755)).unwrap();
    if r.is_ok() {
        eprintln!("running with permission to write anywhere (root?): skipped");
        return;
    }
    assert_eq!(std::fs::read(&fx.file).unwrap(), written);
    assert!(!std::fs::read_dir(&fx.docs).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".sc-tmp")));
    assert_eq!(sess.file_status().unwrap().state, "pending", "still to be written");
    // once writable again the pending edit goes out
    assert_eq!(sess.write_back(&none(), false).unwrap(), Wrote::Written);

    // replace_file itself: a failing rename (target is a folder) keeps the target and leaves no temp
    let dir_target = fx.docs.join("folder.pptx");
    std::fs::create_dir(&dir_target).unwrap();
    assert!(sc_core::file::replace_file(&dir_target, b"x").is_err());
    assert!(dir_target.is_dir());
    assert!(!fx.docs.join(".folder.pptx.sc-tmp").exists());
}

#[test]
fn save_as_binds_an_unbound_deck_and_a_moved_file_finds_its_deck() {
    let fx = setup();
    // an older deck imported by bytes (not bound)
    let name = fx.ws.import_pptx("old", &common::synthetic_deck(), None).unwrap();
    let mut sess = fx.ws.open(&name).unwrap();
    assert!(sess.file_status().is_none());
    let to = fx.docs.join("new talk.pptx");
    sess.save_as(&to, &none()).unwrap();
    assert!(to.is_file());
    assert!(hidden_backups(&fx.docs).is_empty(), "a new file needs no backup");
    assert_eq!(sess.file_status().unwrap().state, "saved");
    assert_eq!(fx.ws.open_path(&to).unwrap(), name);
    drop(sess);

    // moved in Finder: the id in the file leads back to the deck
    let moved = fx.docs.join("moved.pptx");
    std::fs::rename(&to, &moved).unwrap();
    assert_eq!(fx.ws.open_path(&moved).unwrap(), name);
    let sess = fx.ws.open(&name).unwrap();
    assert_eq!(sess.file_status().unwrap().path, sc_core::file::canonical(&moved));
    assert_eq!(sess.file_status().unwrap().state, "saved", "a plain move is no change");
    drop(sess);
    // moved and then edited elsewhere: still asked, never overwritten silently
    let moved2 = fx.docs.join("moved-2.pptx");
    std::fs::rename(&moved, &moved2).unwrap();
    let mut edited = std::fs::read(&moved2).unwrap();
    let od = lore_ooxml::Deck::from_bytes(&edited).unwrap();
    let mut pkg = od.to_package();
    pkg.set("docProps/ellrin.xml", b"<x/>".to_vec());
    edited = pkg.to_bytes().unwrap();
    std::fs::write(&moved2, &edited).unwrap();
    assert_eq!(fx.ws.open_path(&moved2).unwrap(), name);
    assert_eq!(fx.ws.open(&name).unwrap().file_status().unwrap().state, "changed");
    let moved = moved2;
    let sess = fx.ws.open(&name).unwrap();

    // a copy next to it becomes its own deck
    let copy = fx.docs.join("copy.pptx");
    std::fs::copy(&moved, &copy).unwrap();
    let other = fx.ws.open_path(&copy).unwrap();
    assert_ne!(other, name);
    assert_ne!(fx.ws.open(&other).unwrap().file.unwrap().deck_id, sess.file.as_ref().unwrap().deck_id);

    // save-as over an existing file backs it up once first
    let mut s2 = fx.ws.open(&other).unwrap();
    let target = fx.docs.join("talk.pptx");
    let before = std::fs::read(&target).unwrap();
    s2.save_as(&target, &none()).unwrap();
    assert_eq!(std::fs::read(fx.docs.join(".talk.original.pptx")).unwrap(), before);
}

#[cfg(target_os = "macos")]
#[test]
fn replace_file_keeps_xattrs_and_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.pptx");
    std::fs::write(&f, b"old").unwrap();
    std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o640)).unwrap();
    let tag = std::ffi::CString::new("com.apple.metadata:_kMDItemUserTags").unwrap();
    let path = std::ffi::CString::new(f.to_str().unwrap()).unwrap();
    let val = b"bplist-red";
    let rc = unsafe { libc::setxattr(path.as_ptr(), tag.as_ptr(), val.as_ptr() as *const _, val.len(), 0, 0) };
    assert_eq!(rc, 0);
    sc_core::file::replace_file(&f, b"new").unwrap();
    assert_eq!(std::fs::read(&f).unwrap(), b"new");
    assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o640);
    let mut buf = [0u8; 64];
    let n = unsafe { libc::getxattr(path.as_ptr(), tag.as_ptr(), buf.as_mut_ptr() as *mut _, buf.len(), 0, 0) };
    assert_eq!(&buf[..n as usize], val);
}
