//! 潤飾 (lore-polish, `docs/long-paper.md` §A) and the voice (lore-voice) in a paper.
//!
//! | Method | Path | |
//! |---|---|---|
//! | * | /api/polish… | `lore_polish::web::router`: propose, applied, like, history (`doc` = the paper's name) |
//! | * | /api/voice… | `lore_voice::web::router`: hold-to-talk for the 潤飾 dialog |
//!
//! A paper's proposals go to its `draft/polish.jsonl` (`.papercomposer/draft/` for a paper
//! opened where it is), next to 討論草稿's files. The rewrite is locked against the paper's
//! notation registry (its macros), and the paragraph with the rewrite in place goes through
//! lore-notation's checks (`replace_check` → `notation::check` → `check_document`): anything
//! new and blocking (a typed "Table 1", an unknown label…) means it cannot be applied.
//! The author's phrases: the phrase bank entries closest to the passage (`phrases.csv`,
//! `expressions.csv`); his style notes (`style/`, `style_text`) as rules for the rewrite. 👍 also opens 收藏句型 on the rewrite in the page (`/api/phrases/add`).
//!
//! Two presets (寫作三階段, `docs/long-paper.md` §C): 「用字」 (stage 2: word choice only, with
//! the code checks' findings for the passage) and 「我的風格」 (stage 3: toward the author's own
//! style for the passage's section — `pc_core::styleprofile`, from his own rows only, plus 2–3
//! of his exemplars). Neither gets the guide's sentence patterns (`style/patterns.toml`).

use super::*;

pub const LOG: &str = "polish.jsonl";
const PHRASES: usize = 12;
const EXPRESSIONS: usize = 10;

/// The paper's proposal log.
pub fn log_path(paper: &Path) -> PathBuf {
    pc_core::draft::session::draft_dir(paper).join(LOG)
}

/// The phrase bank entries most useful for `text`.
pub fn phrases_for(profile: Option<&Profile>, text: &str) -> Vec<String> {
    let Some(p) = profile else { return vec![] };
    // the author's voice only: a row marked as guide material never goes in
    let guide = |tags: &[String]| tags.iter().any(|t| pc_core::styleprofile::is_guide_mark(t));
    let mut out: Vec<String> = search(&p.phrases, &Query { text: Some(text.to_string()), limit: Some(PHRASES + 4), ..Default::default() })
        .into_iter()
        .filter(|h| !guide(&h.phrase.tags))
        .take(PHRASES)
        .map(|h| h.phrase.template)
        .collect();
    let mut ex: Vec<(f64, &pc_core::Expression)> = p.expressions.iter().filter(|e| !guide(&e.tags)).map(|e| (lore_polish::liked::similarity(&format!("{} {}", e.text, e.example), text), e)).collect();
    ex.sort_by(|a, b| b.0.total_cmp(&a.0));
    out.extend(ex.into_iter().take(EXPRESSIONS).map(|(_, e)| if e.usage.trim().is_empty() { e.text.clone() } else { format!("{} ({})", e.text, e.usage.trim()) }));
    out
}

/// What lore-polish needs to know about a paper.
pub fn doc_for(workspace: Option<&Path>, paper: &Path, profile: Option<&Profile>, text: &str) -> lore_polish::Doc {
    let n = crate::notation::load(workspace.unwrap_or(paper), paper);
    lore_polish::Doc {
        log: log_path(paper),
        macros: lore_polish::lock::macros_of(&n),
        language: profile.map(|p| p.manifest.language.clone()).unwrap_or_default(),
        phrases: if text.is_empty() { vec![] } else { phrases_for(profile, text) },
        tone: String::new(),
        // the author's style notes (style/), rules for the rewrite
        style: workspace.map(pc_core::draft::style_text).unwrap_or_default(),
    }
}

/// lore-notation's findings the rewrite adds to its paragraph, blocking ones only.
pub fn check_for(workspace: Option<&Path>, paper: &Path, before: &str, after: &str) -> Vec<String> {
    let r = replace_check(workspace.unwrap_or(paper), paper, &[before.to_string()], &[after.to_string()], "");
    r["blocking"].as_array().map(|a| a.iter().filter_map(|m| m.as_str().map(|m| format!("對照表：{m}"))).collect()).unwrap_or_default()
}

pub const DICTION: &str = "diction";
pub const VOICE: &str = "voice";

/// 「用字」 and 「我的風格」.
pub fn presets() -> Vec<lore_polish::Preset> {
    vec![
        lore_polish::Preset {
            id: DICTION.into(),
            label: "用字".into(),
            instruction: "Word choice only (writing stage 2): replace wordy, vague, hedged, jargon or informal words and the ones the code checks found below (use their suggestions where they fit), following the author's writing rules. Keep every sentence, its structure and the order of sentences and clauses as they are; add or remove no content."
                .into(),
        },
        lore_polish::Preset {
            id: VOICE.into(),
            label: "我的風格".into(),
            instruction: "The author's own style (writing stage 3): reshape the passage toward his style for this section described below — how his paragraphs open and close, the order of his argument, the words his closing sentences start with, how much of the paragraph the main claim takes and where it sits, his typical lengths. Keep every fact, number and claim; add no content. His paragraphs show the moves to follow, never content or sentences to copy."
                .into(),
        },
    ]
}

/// What a preset adds for a passage: the code checks' findings (用字), or the author's style
/// for its section and a few of his own paragraphs (我的風格). `cache_root`: the style
/// profile's cache tier.
pub fn preset_block_for(workspace: Option<&Path>, profile: Option<&Profile>, preset: &str, p: &lore_polish::PresetPassage, cache_root: &Path) -> String {
    let section = pc_core::patterns::canonical_section(p.scope);
    match preset {
        DICTION => {
            let Some(prof) = profile else { return String::new() };
            let mut lines: Vec<String> = vec![];
            for f in pc_core::check_text(prof, p.text, Some(&section).filter(|s| !s.is_empty()).map(String::as_str)).iter().flat_map(|r| &r.findings) {
                // word-level findings only (the message carries “found” → “suggested”)
                if f.found.is_none() {
                    continue;
                }
                let l = format!("- {}", f.message);
                if !lines.contains(&l) {
                    lines.push(l);
                }
            }
            if lines.is_empty() {
                "The author's code checks found nothing in this passage.".into()
            } else {
                format!("The author's code checks found (found → suggested):\n{}", lines.join("\n"))
            }
        }
        VOICE => {
            let (Some(ws), Some(prof)) = (workspace, profile) else { return String::new() };
            let sp = pc_core::styleprofile::load_or_build(&prof.phrases, &prof.expressions, &prof.root, ws, &pc_core::styleprofile::cache_file(cache_root, ws));
            let mut out = sp.prompt_text(&section);
            if out.is_empty() {
                return "No style profile for this section yet (no paragraphs of the author's own in it): change only what his writing rules ask for.".into();
            }
            let all = pc_core::styleprofile::read_exemplars(&prof.root);
            let (guide, _) = pc_core::patterns::load(ws);
            let own = pc_core::styleprofile::own(&prof.phrases, &prof.expressions, &all, &guide);
            let picked = pc_core::styleprofile::exemplars_for(&own.exemplars, &section, "", p.text);
            if !picked.is_empty() {
                out += "\nHis own paragraphs in this section (follow their moves; never copy their content or sentences):\n";
                for e in picked {
                    out += &format!("- [{}] {}\n", e.function, pc_core::styleprofile::cut(&e.text));
                }
            }
            out
        }
        _ => String::new(),
    }
}

/// The style profile's cache tier (wipeable; rebuilt from the profile).
pub fn cache_root() -> PathBuf {
    dirs::cache_dir().unwrap_or_else(std::env::temp_dir).join("PaperComposer")
}

struct PcHost(Shared);

#[async_trait::async_trait]
impl lore_polish::Host for PcHost {
    fn app(&self) -> &str {
        "PaperComposer"
    }
    fn latex(&self) -> bool {
        true
    }
    fn home(&self) -> lore_home::Home {
        loreforge_home()
    }
    async fn doc(&self, doc: &str, text: &str) -> Result<lore_polish::Doc, String> {
        let dir = paper_dir(&self.0, doc).await.map_err(|_| format!("找不到論文 {doc}"))?;
        let l = self.0.loaded.read().await;
        Ok(doc_for(l.workspace.as_deref(), &dir, l.profile.as_ref(), text))
    }
    async fn complete(&self, req: lore_llm::LlmRequest, engine: Option<&str>) -> Result<(serde_json::Value, String), String> {
        let choice: Option<EngineChoice> = engine.filter(|e| !e.is_empty() && *e != "auto").and_then(|e| serde_json::from_value(serde_json::json!(e)).ok());
        let route = self.0.llm.route(lore_polish::TASK, choice).await?;
        let req = route.apply(req.temperature(llm::LOCAL_TEMPERATURE));
        let v = lore_llm::complete_json(route.provider.as_ref(), &req).await.map_err(|e| format!("模型沒有回答：{e}"))?;
        let id = route.provider.id();
        lore_llm::shared::record_used(lore_polish::TASK, &id);
        Ok((v, id))
    }
    async fn check(&self, doc: &str, before: &str, after: &str) -> Vec<String> {
        let Ok(dir) = paper_dir(&self.0, doc).await else { return vec![] };
        let ws = self.0.loaded.read().await.workspace.clone();
        tokio::task::spawn_blocking({
            let (b, a) = (before.to_string(), after.to_string());
            move || check_for(ws.as_deref(), &dir, &b, &a)
        })
        .await
        .unwrap_or_default()
    }
    fn presets(&self) -> Vec<lore_polish::Preset> {
        presets()
    }
    async fn preset_block(&self, _doc: &str, preset: &str, p: &lore_polish::PresetPassage<'_>) -> String {
        let l = self.0.loaded.read().await;
        let cache = if cfg!(test) { std::env::temp_dir().join("pc-test-cache") } else { cache_root() };
        preset_block_for(l.workspace.as_deref(), l.profile.as_ref(), preset, p, &cache)
    }
}

/// The voice: the system's on macOS; none in tests and the isolated example.
pub(crate) fn voice() -> lore_voice::Voice {
    if cfg!(test) || engines_isolated() { lore_voice::Voice::unavailable("這裡沒有語音") } else { lore_voice::Voice::system() }
}

pub(crate) fn routes(state: &Shared, voice: lore_voice::Voice) -> Router<Shared> {
    let def = lore_voice::SpeakRequest { language: "en-US".into(), ..Default::default() };
    lore_polish::web::router(Arc::new(PcHost(state.clone()))).merge(lore_voice::web::router(voice, Arc::new(def)))
}

/// What PaperComposer asks for at its first launch (設定 → 一般 → 權限 afterwards): the voice's
/// three (潤飾's hold-to-talk). Tests and the isolated example: never asked, nothing opened.
pub(crate) fn permissions(voice: &lore_voice::Voice) -> lore_shell::permissions::Permissions {
    use lore_shell::permissions::{Permission, Permissions};
    let p = Permissions::new(&[Permission::Microphone, Permission::Speech, Permission::PersonalVoice]).with(Arc::new(voice.clone()));
    if cfg!(test) || engines_isolated() { p.isolated() } else { p }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The paper's own host over a temp folder, the model replaced by canned replies.
    struct Fake {
        ws: PathBuf,
        dir: PathBuf,
        home: lore_home::Home,
        replies: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl lore_polish::Host for Fake {
        fn app(&self) -> &str {
            "PaperComposer"
        }
        fn home(&self) -> lore_home::Home {
            self.home.clone()
        }
        async fn doc(&self, _doc: &str, text: &str) -> Result<lore_polish::Doc, String> {
            Ok(doc_for(Some(&self.ws), &self.dir, None, text))
        }
        async fn complete(&self, _req: lore_llm::LlmRequest, _engine: Option<&str>) -> Result<(serde_json::Value, String), String> {
            Ok((serde_json::json!({ "rewrite": self.replies.lock().unwrap().remove(0) }), "mock".into()))
        }
        async fn check(&self, _doc: &str, before: &str, after: &str) -> Vec<String> {
            check_for(Some(&self.ws), &self.dir, before, after)
        }
    }

    fn paper(root: &Path, in_place: bool) -> (PathBuf, PathBuf) {
        let (ws, dir) = (root.join("ws"), root.join("ws/papers/P"));
        std::fs::create_dir_all(dir.join("sections")).unwrap();
        if in_place {
            std::fs::create_dir_all(dir.join(".papercomposer")).unwrap();
        }
        std::fs::write(dir.join("main.tex"), "\\documentclass{article}\n\\begin{document}\n\\input{sections/res}\n\\end{document}\n").unwrap();
        std::fs::write(
            dir.join("sections/res.tex"),
            "\\section{Results}\nMoreover, Table~\\ref{tab:main} shows that \\vw gains $x$ over \\cite{a2020}, which is very good.\n\n\\begin{table}\n\\caption{Main.}\n\\label{tab:main}\n\\begin{tabular}{l}\nx\n\\end{tabular}\n\\end{table}\n",
        )
        .unwrap();
        std::fs::write(
            pc_core::paper::app_dir(&dir).join("notation.json"),
            r#"{"version":1,"entries":[{"id":"t1","kind":"table","label":"tab:main","caption":"Main"},{"id":"w","kind":"symbol","latex":"\\boldsymbol{w}","macro":"\\vw","meaning":"weights"}]}"#,
        )
        .unwrap();
        (ws, dir)
    }

    fn body(text: &str, prefix: &str, suffix: &str) -> lore_polish::ProposeBody {
        lore_polish::ProposeBody { doc: "P".into(), text: text.into(), prefix: prefix.into(), suffix: suffix.into(), instruction: "drop Moreover; the ending is wordy".into(), ..Default::default() }
    }

    #[test]
    fn the_style_notes_reach_the_polish_prompt() {
        let root = tempfile::tempdir().unwrap();
        let (ws, dir) = paper(root.path(), false);
        assert!(doc_for(Some(&ws), &dir, None, "x").style.is_empty(), "no style folder, no notes");
        std::fs::create_dir_all(ws.join("style")).unwrap();
        std::fs::write(ws.join("style/10-writing.md"), "No contractions.").unwrap();
        std::fs::write(ws.join("style/checks.toml"), "wordy = [\"in order to\"]").unwrap();
        let doc = doc_for(Some(&ws), &dir, None, "x");
        assert!(doc.style.contains("No contractions.") && !doc.style.contains("in order to"));
        let req = lore_polish::prompt::request(&lore_polish::prompt::Input { passage: "x", instruction: "shorter", style: &doc.style, ..Default::default() });
        assert!(req.flatten().contains("style rules (follow them in the rewrite; rules, not content):"));
    }

    #[tokio::test]
    async fn a_rewrite_is_locked_checked_and_logged_in_the_draft_folder() {
        for in_place in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let (ws, dir) = paper(root.path(), in_place);
            let doc = doc_for(Some(&ws), &dir, None, "x");
            assert!(doc.macros.contains(&"vw".to_string()), "{:?}", doc.macros);
            let log = log_path(&dir);
            assert_eq!(log, if in_place { dir.join(".papercomposer/draft/polish.jsonl") } else { dir.join("draft/polish.jsonl") });

            let text = "Moreover, Table~\\ref{tab:main} shows that \\vw gains $x$ over \\cite{a2020}, which is very good.";
            let h = Fake {
                ws: ws.clone(),
                dir: dir.clone(),
                home: lore_home::Home::at(root.path()),
                replies: Mutex::new(vec![
                    // the registered macro, the reference, the math and the citation all come back
                    "Table~⟦1⟧ shows that ⟦2⟧ gains ⟦3⟧ over ⟦4⟧.".into(),
                    // the model typed the table number and dropped the reference
                    "Table 1 shows that ⟦2⟧ gains ⟦3⟧ over ⟦4⟧.".into(),
                    // all placeholders kept, but a typed number added: lore-notation refuses
                    "Table~⟦1⟧ (Table 1) shows that ⟦2⟧ gains ⟦3⟧ over ⟦4⟧.".into(),
                ]),
            };
            let p = lore_polish::propose(&h, body(text, "", "")).await.unwrap();
            assert!(p.ok, "{:?}", p.problems);
            assert_eq!(p.after, "Table~\\ref{tab:main} shows that \\vw gains $x$ over \\cite{a2020}.");
            // lore-notation's check_document on the paragraph as it will be: nothing blocking
            let n = crate::notation::load(&ws, &dir);
            let rep = lore_notation::check_document(lore_notation::Document::from_files(&[("main.tex".to_string(), format!("{}\n\\begin{{table}}\\caption{{Main.}}\\label{{tab:main}}\\end{{table}}\n", p.after))]), &n, &Default::default());
            let codes: Vec<String> = serde_json::to_value(&rep).unwrap()["issues"].as_array().map(|a| a.iter().filter_map(|i| i["code"].as_str().map(str::to_string)).collect()).unwrap_or_default();
            assert!(!codes.iter().any(|c| c == "hardcoded_reference" || c == "undefined_ref"), "{codes:?}");

            let p = lore_polish::propose(&h, body(text, "", "")).await.unwrap();
            assert!(!p.ok && p.problems.contains(&"少了 \\ref{tab:main}".to_string()), "{:?}", p.problems);
            let p = lore_polish::propose(&h, body(text, "", "")).await.unwrap();
            assert!(!p.ok && p.problems.iter().any(|m| m.starts_with("對照表：")), "a typed table number is blocking: {:?}", p.problems);

            let lines = lore_polish::log::read(&log);
            assert_eq!(lines.len(), 3);
            assert!(lines.iter().all(|e| e.before == text && e.engine == "mock"));
            // 👍 lands only in the temp home's private/shared
            lore_polish::like(&h, "P", &lines[0].id).await.unwrap();
            assert!(root.path().join("private/shared/polish/liked.jsonl").is_file());
        }
    }

    #[test]
    fn the_polish_task_is_a_create_task_claude_first() {
        let t = Llm::task_table(None).unwrap();
        let spec = &t.task[lore_polish::TASK];
        assert_eq!((spec.class, spec.default.as_str(), spec.fallback.as_deref()), (Some(lore_llm::TaskClass::Create), "claude:selected", Some("local:current")));
    }

    /// A profile in memory over `root/profile`: the author's rows, one guide row that slipped
    /// in, a wordy list turned on by a hard rule.
    fn author_profile(root: &Path) -> Profile {
        use pc_core::profile::{Manifest, Phrase, Rule, RuleKind};
        let dir = root.join("ws/profile");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("exemplars.csv"),
            "id,section,function,text,source\n\
E1,Result,finding,\"The proposed loss raised recall on long inputs. These results indicate that attention stays on the lesion.\",\"Author, Journal (2024)\"\n\
E2,Result,comparison,\"Although trains and buses differ in speed, both reduce traffic in city centres.\",guide\n",
        )
        .unwrap();
        let ph = |id: &str, pos: &str, t: &str, tags: &[&str]| Phrase { id: id.into(), section: "Result".into(), function: "finding".into(), position: pos.into(), template: t.into(), example: String::new(), tags: tags.iter().map(|s| s.to_string()).collect(), priority: 3 };
        let (checks, _) = pc_core::stylecheck::StyleChecks::parse("wordy = [{ find = \"in order to\", use = \"to\" }]");
        Profile {
            root: dir,
            authors: vec![],
            affiliations: vec![],
            style_checks: checks,
            manifest: Manifest { name: "t".into(), schema_version: 1, language: "en".into() },
            phrases: vec![
                ph("P1", "opening", "Compared with {baseline}, {method} improved {metric}.", &["paper1"]),
                ph("P2", "closing", "These results indicate that {claim}.", &["paper1"]),
                ph("G1", "opening", "{topic} is superior to {other} because {reason}.", &["guide"]),
            ],
            expressions: vec![],
            vocabulary: vec![],
            rules: vec![Rule { id: "R201".into(), kind: RuleKind::Hard, scope: "paragraph".into(), check: "phrases".into(), threshold: "wordy".into(), severity: "warning".into(), message: String::new() }],
        }
    }

    struct PresetFake {
        ws: PathBuf,
        dir: PathBuf,
        home: lore_home::Home,
        profile: Profile,
        cache: PathBuf,
        seen: Mutex<Vec<String>>,
        replies: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl lore_polish::Host for PresetFake {
        fn app(&self) -> &str {
            "PaperComposer"
        }
        fn home(&self) -> lore_home::Home {
            self.home.clone()
        }
        async fn doc(&self, _doc: &str, text: &str) -> Result<lore_polish::Doc, String> {
            Ok(doc_for(Some(&self.ws), &self.dir, Some(&self.profile), text))
        }
        async fn complete(&self, req: lore_llm::LlmRequest, _engine: Option<&str>) -> Result<(serde_json::Value, String), String> {
            self.seen.lock().unwrap().push(req.flatten());
            Ok((serde_json::json!({ "rewrite": self.replies.lock().unwrap().remove(0) }), "mock".into()))
        }
        async fn check(&self, _doc: &str, before: &str, after: &str) -> Vec<String> {
            check_for(Some(&self.ws), &self.dir, before, after)
        }
        fn presets(&self) -> Vec<lore_polish::Preset> {
            presets()
        }
        async fn preset_block(&self, _doc: &str, preset: &str, p: &lore_polish::PresetPassage<'_>) -> String {
            preset_block_for(Some(&self.ws), Some(&self.profile), preset, p, &self.cache)
        }
    }

    #[tokio::test]
    async fn the_two_presets_bring_findings_or_his_own_style_never_the_guide() {
        let root = tempfile::tempdir().unwrap();
        let (ws, dir) = paper(root.path(), false);
        std::fs::create_dir_all(ws.join("style")).unwrap();
        std::fs::write(ws.join("style/10-writing.md"), "No contractions.").unwrap();
        let guide = "Although trains and buses differ in speed, both reduce traffic in city centres.";
        std::fs::write(ws.join("style/patterns.toml"), format!("[[pattern]]\nid = \"g\"\nsituation = \"compare_two\"\nstructure = \"Although {{A}} and {{B}} differ in {{x}}, both {{y}}.\"\nexample = \"{guide}\"\nsource = \"guide\"\n")).unwrap();
        let h = PresetFake {
            ws: ws.clone(),
            dir: dir.clone(),
            home: lore_home::Home::at(root.path()),
            profile: author_profile(root.path()),
            cache: root.path().join("cache"),
            seen: Mutex::new(vec![]),
            replies: Mutex::new(vec!["To check it, Table~⟦1⟧ shows that ⟦2⟧ gains ⟦3⟧ over ⟦4⟧.".into(), "Table~⟦1⟧ shows that ⟦2⟧ gains ⟦3⟧ over ⟦4⟧.".into()]),
        };
        let text = "In order to check it, Table~\\ref{tab:main} shows that \\vw gains $x$ over \\cite{a2020}.";
        let mut b = lore_polish::ProposeBody { doc: "P".into(), text: text.into(), scope: "4. Results".into(), preset: Some(DICTION.into()), ..Default::default() };
        let p = lore_polish::propose(&h, b.clone()).await.unwrap();
        assert!(p.ok, "{:?}", p.problems);
        assert_eq!(p.after, "To check it, Table~\\ref{tab:main} shows that \\vw gains $x$ over \\cite{a2020}.");
        let sent = h.seen.lock().unwrap()[0].clone();
        assert!(sent.contains("For this rewrite (用字):") && sent.contains("\u{201c}in order to\u{201d} \u{2192} \u{201c}to\u{201d}"), "{sent}");
        assert!(sent.contains("Word choice only") && sent.contains("No contractions."));
        assert!(!sent.contains("\\cite{a2020}") && !sent.contains("$x$"), "LaTeX still locked");
        // 我的風格: his profile and his paragraph; nothing of the guide
        b.preset = Some(VOICE.into());
        let p = lore_polish::propose(&h, b).await.unwrap();
        assert!(p.ok, "{:?}", p.problems);
        let sent = h.seen.lock().unwrap()[1].clone();
        assert!(sent.contains("For this rewrite (我的風格):") && sent.contains("The author's own style in Result paragraphs"), "{sent}");
        assert!(sent.contains("These results indicate that {claim}.") && sent.contains("The proposed loss raised recall"));
        for never in [guide, "trains and buses", "superior to", pc_core::patterns::HEADING] {
            assert!(!sent.contains(never), "guide material in a stage-3 prompt: {never}");
        }
        // liked: the label, never the block or the guide
        let id = lore_polish::log::read(&log_path(&dir))[1].id.clone();
        lore_polish::like(&h, "P", &id).await.unwrap();
        let lib = std::fs::read_to_string(root.path().join("private/shared/polish/liked.jsonl")).unwrap();
        assert!(lib.contains("「我的風格」") && !lib.contains("trains") && !lib.contains("typical openings"));
        assert_eq!(presets().iter().map(|p| p.label.as_str()).collect::<Vec<_>>(), ["用字", "我的風格"]);
    }
}
