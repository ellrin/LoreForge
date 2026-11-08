//! Run the new-paper path (outline + authors) on every template in a
//! publishers folder. Not run by default: it needs a workspace.
//!
//!   PAPERCOMPOSER_PUBLISHERS=/path/to/private/publishers cargo test -p pc-core --test templates -- --ignored --nocapture

use pc_core::authors::{self, Affiliation, Author, PaperAuthor};
use pc_core::meta::{OutlineSection, OutlineSub};
use pc_core::paper;

fn balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 1,
            b'%' => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
        i += 1;
    }
    depth == 0
}

#[test]
#[ignore]
fn every_template_takes_an_outline_and_authors() {
    let Ok(root) = std::env::var("PAPERCOMPOSER_PUBLISHERS") else { return };
    let out = std::env::temp_dir().join(format!("pc-templates-{}", std::process::id()));
    std::fs::create_dir_all(&out).unwrap();
    let people = vec![
        Author { id: "A001".into(), given: "Shang-Lin".into(), family: "Chung".into(), email: "me@u.edu".into(), orcid: "0000-0001-2345-6789".into(), affiliations: vec!["F001".into()], ..Default::default() },
        Author { id: "A002".into(), given: "Mei".into(), family: "Lin".into(), email: "mei@h.org".into(), affiliations: vec!["F002".into(), "F001".into()], ..Default::default() },
    ];
    let affils = vec![
        Affiliation { id: "F001".into(), department: "Department of Computer Science".into(), organization: "National University".into(), city: "Taipei".into(), country: "Taiwan".into() },
        Affiliation { id: "F002".into(), department: String::new(), organization: "City Hospital".into(), city: String::new(), country: "Taiwan".into() },
    ];
    let chosen = vec![PaperAuthor { id: "A001".into(), corresponding: true, equal: false }, PaperAuthor { id: "A002".into(), corresponding: false, equal: false }];
    let (p, a) = authors::resolve(&chosen, &people, &affils).unwrap();
    let outline = vec![
        OutlineSection { title: "Introduction".into(), intent: "Why fracture reading needs explanations.".into(), subsections: vec![] },
        OutlineSection { title: "Methods".into(), intent: "The model and data.".into(), subsections: vec![OutlineSub { title: "Dataset".into(), intent: "Where images came from.".into() }] },
        OutlineSection { title: "Discussion".into(), intent: "What the gain means.".into(), subsections: vec![] },
    ];
    let mut dirs: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().filter(|e| e.path().join("sample.tex").is_file()).map(|e| e.path()).collect();
    dirs.sort();
    let mut failed = Vec::new();
    for t in dirs {
        let name = t.file_name().unwrap().to_string_lossy().to_string();
        let dir = match paper::create_paper_from_template_with(&out, &name, &t, Some(&outline)) {
            Ok(d) => d,
            Err(e) => {
                failed.push(format!("{name}: {e}"));
                continue;
            }
        };
        let main = std::fs::read_to_string(dir.join("main.tex")).unwrap();
        let style = authors::detect(&main);
        let after = authors::apply(&main, &p, &a);
        std::fs::write(dir.join("main.tex"), &after).unwrap();
        let files = paper::section_files(&dir);
        let mut problems = Vec::new();
        if !balanced(&after) {
            problems.push("unbalanced braces".to_string());
        }
        for needle in ["Chung", "Lin", "National University", "City Hospital"] {
            if !after.contains(needle) {
                problems.push(format!("missing {needle}"));
            }
        }
        if after.matches("Chung").count() > 3 {
            problems.push("author block written twice?".into());
        }
        if files.len() != 3 || !files[1].contains("methods") {
            problems.push(format!("sections: {files:?}"));
        }
        let methods = std::fs::read_to_string(dir.join("sections").join(&files[1])).unwrap_or_default();
        if !methods.contains("\\subsection{Dataset}") {
            problems.push("no subsection".into());
        }
        // the sample's placeholder people must be gone from the title block
        // (comments and the sample's biographies may still name them)
        let code: String = after.lines().map(|l| if l.trim_start().starts_with('%') { "" } else { l }).collect::<Vec<_>>().join("\n");
        let front = &code[..code.find("\\begin{IEEEbiography").unwrap_or(code.len())];
        for ghost in ["First Author", "Michael Shell", "Michael~Shell", "Hippocampus", "\\fnm{First}"] {
            if front.contains(ghost) {
                problems.push(format!("sample author left: {ghost}"));
            }
        }
        let start = after.find("Chung").map(|i| after[..i].rfind('\n').unwrap_or(0)).unwrap_or(0);
        let block: String = after[start..].lines().take(14).collect::<Vec<_>>().join("\n");
        println!("===== {name}  {style:?}\n{block}\n");
        if !problems.is_empty() {
            failed.push(format!("{name}: {}", problems.join("; ")));
        }
    }
    if failed.is_empty() {
        std::fs::remove_dir_all(&out).ok();
    }
    assert!(failed.is_empty(), "\n{}\n(papers kept in {})", failed.join("\n"), out.display());
}
