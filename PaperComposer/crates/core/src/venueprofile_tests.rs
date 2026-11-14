use super::*;

/// `n` filler words (synthetic, no real text).
fn filler(n: usize, tag: &str) -> String {
    (0..n).map(|i| format!("{tag}{}", i % 7)).collect::<Vec<_>>().join(" ")
}

fn sec(level: u8, title: &str, paras: Vec<String>) -> SecText {
    SecText { level, title: title.into(), paragraphs: paras }
}

/// A synthetic article: introduction (value, prior with citations, a bulleted contribution
/// list), optional related work, methods (prose + equations), results, discussion with
/// limitations, conclusion, data availability. Words are fixed so the shares are known.
fn article(i: usize, related: bool) -> ArticleText {
    let mut s = vec![
        sec(1, "I. INTRODUCTION", vec![
            format!("Screening matters for patients. {}", filler(98, "val")),
            format!("Earlier widget models [1], [2]-[4] were slow. {}", filler(93, "pri")),
            format!("To the best of our knowledge this is new. Our contributions: • a widget • a demo {}", filler(84, "con")),
        ]),
    ];
    if related {
        s.push(sec(1, "II. RELATED WORK", vec![format!("Prior widgets [5]. {}", filler(97, "rel"))]));
    }
    s.extend([
        sec(1, "Materials and Methods", vec![format!("We propose the widget network. {}", filler(195, "met")), format!("The loss is defined as follows. {}", filler(95, "equ"))]),
        sec(2, "Dataset", vec![format!("The demo cohort has scans. {}", filler(95, "dat"))]),
        sec(1, "Results", vec![format!("The proposed method achieves high scores. {}", filler(195, "res"))]),
        sec(1, "Discussion", vec![format!("The proposed method works well but has weaknesses. {}", filler(91, "dis")), format!("A limitation of this study is size. {}", filler(92, "lim"))]),
        sec(1, "Conclusion", vec![format!("In conclusion the widget helps. {}", filler(94, "cnc"))]),
        sec(1, "Data availability", vec!["The data are available on request.".into()]),
    ]);
    let methods = s.iter().position(|x| x.title.contains("Methods")).unwrap();
    ArticleText {
        doi: format!("10.5555/demo.{i}"),
        title: format!("Synthetic demo article {i}"),
        year: Some(2026),
        licence: "cc-by".into(),
        url: format!("https://example.org/{i}.pdf"),
        copy: "publisher".into(),
        sections: s,
        captions: vec![Caption { kind: "figure".into(), number: "1".into(), text: "Overview of the proposed pipeline.".into() }, Caption { kind: "table".into(), number: "1".into(), text: "Scores.".into() }],
        equations: vec![Some(methods), Some(methods), None],
    }
}

/// What a model would answer for [`article`].
fn reply(a: &ArticleText) -> Value {
    let mut paras = vec![];
    for (p, (_, text)) in a.paragraphs().iter().enumerate() {
        let c = match &text[..5] {
            "Scree" => "value",
            "Earli" | "Prior" => "prior",
            "To th" => "contribution",
            "We pr" => "method",
            "The l" => "equations",
            "The d" if text.contains("cohort") => "dataset",
            "The p" if text.contains("achieves") => "results",
            "The p" => "discussion",
            "A lim" => "limitations",
            "In co" => "conclusion",
            _ => "statements",
        };
        paras.push(json!({ "p": p, "c": c }));
    }
    json!({ "sections": [{ "s": 0, "role": "introduction" }], "paragraphs": paras, "points": ["pros_cons", "made_up"], "extra": [] })
}

fn five() -> Vec<(ArticleText, Labels)> {
    (0..5)
        .map(|i| {
            let a = article(i, i < 3);
            let l = parse_labels(&reply(&a), &a, &[]);
            (a, l)
        })
        .collect()
}

fn ident() -> Identity {
    Identity { openalex: "S900001".into(), name: "Journal of Demo Imaging".into(), issn_l: "1234-5678".into(), ..Default::default() }
}

#[test]
fn headings_names_and_dates() {
    assert_eq!(canonical_role("2. Materials and Methods"), "methods");
    assert_eq!(canonical_role("II. RELATED WORK"), "related");
    assert_eq!(canonical_role("Results and Discussion"), "results");
    assert_eq!(canonical_role("Data availability"), "statements");
    assert_eq!(canonical_role("Limitations"), "limitations");
    assert_eq!(canonical_role("4.1 Dataset"), "data");
    assert_eq!(clean_title("III. MATERIALS AND METHODS"), "Materials and Methods");
    assert_eq!(clean_title("2.1 Data collection"), "Data collection");
    assert_eq!(search_name("ICLR 2027"), "ICLR");
    assert_eq!(search_name("Elsevier CAS（通用，雙欄）"), "Elsevier CAS");
    assert_eq!(search_name("Demo/Society DCV 2027"), "Demo/Society DCV");
    assert_eq!(date_of(day_number("2026-11-14").unwrap()), "2026-11-14");
    assert_eq!(from_date("2026-11-14"), "2024-11-14");
    assert!(!is_stale("2025-11-20", "2026-11-14"));
    assert!(is_stale("2025-11-01", "2026-11-14"));
    assert!(is_stale("not a date", "2026-11-14"));
    assert!(hidden_role("base", false, true) && hidden_role("umbrella", false, false) && hidden_role("", true, false) && !hidden_role("", false, false) && !hidden_role("journal", true, false) && !hidden_role("", true, true));
}

#[test]
fn citations_are_counted_in_both_styles() {
    assert_eq!(citations("as in [1], [2]-[4] and [5, 7–9]"), 1 + 1 + 1 + 4);
    assert_eq!(citations("as shown (Example et al., 2021; Other, 2019)"), 2);
    assert_eq!(citations("no marks [a] here"), 0);
    // intervals and counts are not citations
    assert_eq!(citations("values in [0, 1] from 20 patients, 20 controls and 2021 scans"), 0);
    assert_eq!(citations("as Example et al. (2020) showed"), 1);
}

#[test]
fn bullets_are_items_not_years_or_figures() {
    assert!(bulleted(&["Our contributions are: 1) a widget; 2) a demo."]));
    assert!(bulleted(&["We (i) build and (ii) test it."]));
    assert!(bulleted(&["• one • two"]));
    assert!(!bulleted(&["As shown before (Example et al., 2021) and (Other, 2022), see Fig. 1) and Fig. 2)."]));
    assert!(!bulleted(&["The loss in Eq. (1) and Eq. (2) is minimised."]));
    assert!(!bulleted(&["Only 1) one item."]));
}

#[test]
fn a_paper_made_from_the_skeleton_passes_the_notation_checks() {
    use lore_notation::{CheckOptions, Document, Notation, check_document};
    let mut p = build("demo-journal", &ident(), "article", "2026-11-14", "test", &five());
    // headings as PDFs give them: typographic punctuation, specials, a stray symbol
    p.sections[0].titles[0].text = "Introduction – Why “Widgets” Matter & Cost ☃".into();
    let sk = skeleton(&p);
    assert_eq!(sk[0].title, "Introduction -- Why \"Widgets\" Matter \\& Cost");
    assert!(sk.iter().all(|s| s.intent.is_empty() && s.subsections.iter().all(|x| x.intent.is_empty())));
    let d = tempfile::tempdir().unwrap();
    let tpl = d.path().join("tpl");
    std::fs::create_dir_all(&tpl).unwrap();
    std::fs::write(tpl.join("sample.tex"), "\\documentclass{article}\n\\begin{document}\n\\title{T}\n\\maketitle\n\\section{Introduction}\nSample text.\n\\section{Method}\nMore.\n\\end{document}\n").unwrap();
    let dir = crate::paper::create_paper_from_template_with(&d.path().join("papers"), "skel", &tpl, Some(&sk)).unwrap();
    let mut files = vec![("main.tex".to_string(), std::fs::read_to_string(dir.join("main.tex")).unwrap())];
    for f in crate::paper::section_files(&dir) {
        files.push((format!("sections/{f}"), std::fs::read_to_string(dir.join("sections").join(&f)).unwrap()));
    }
    let all: String = files.iter().map(|f| f.1.clone()).collect();
    assert!(all.contains("Materials and Methods") && all.contains("\\subsection{Dataset}") && !all.contains("Sample text"), "{all}");
    assert!(all.is_ascii(), "nothing a pdflatex class would choke on");
    let issues = check_document(Document::from_files(&files), &Notation::default(), &CheckOptions::default()).issues;
    assert!(!issues.iter().any(|i| ["hardcoded_reference", "ref_undefined"].iter().any(|c| i.code.starts_with(c)) || i.code.starts_with("label_")), "{:?}", issues.iter().map(|i| &i.code).collect::<Vec<_>>());
}

#[test]
fn labels_are_checked_against_the_article() {
    let a = article(0, true);
    let n = a.paragraphs().len();
    // unknown category, a hole, an extra that does not start with x-, one that does
    let v = json!({
        "sections": [{ "s": 1, "role": "related" }, { "s": 99, "role": "methods" }, { "s": 2, "role": "nonsense" }],
        "paragraphs": [{ "p": 0, "c": "value" }, { "p": 1, "c": "nonsense" }, { "p": 2, "c": "x-trial" }],
        "extra": [{ "id": "trial", "label": "試驗" }, { "id": "x-trial", "label": "臨床試驗設計" }],
        "points": ["pros_cons", "made_up"]
    });
    let l = parse_labels(&v, &a, &[]);
    assert_eq!(l.cats.len(), n);
    assert_eq!(l.cats[0], "value");
    assert_eq!(l.cats[1], "value"); // filled from the previous one
    assert_eq!(l.cats[2], "x-trial");
    assert!(l.cats[3..].iter().all(|c| c == "x-trial"));
    assert_eq!(l.extra.len(), 1);
    assert_eq!(l.points, ["pros_cons"]);
    assert_eq!(l.roles[1], "related");
    assert_eq!(l.roles[2], "methods"); // the heading's own role stays
    // a known category from an earlier article is accepted
    let known = vec![Category { id: "x-trial".into(), label: "臨床試驗設計".into(), ..Default::default() }];
    let (sys, user, schema) = label_request(&a, &known);
    assert!(sys.contains("x-trial") && sys.contains("- value:") && user.contains("[P0]") && user.contains("[S0]"));
    assert_eq!(schema["required"][0], "sections");
    assert_eq!(parse_labels(&json!({ "paragraphs": [{ "p": 0, "c": "x-trial" }] }), &a, &known).cats[0], "x-trial");
}

#[test]
fn proportions_habits_and_counts_come_from_code() {
    let arts = five();
    let p = build("demo-journal", &ident(), "article", "2026-11-14", "test", &arts);
    assert_eq!(p.articles.len(), 5);
    // body words: intro 300 + related 100 (3 of 5) + methods 300 + dataset 100 + results 200 + discussion 200 + conclusion 100
    assert_eq!(p.articles[0].words, 1300);
    assert_eq!(p.articles[4].words, 1200);
    let intro = p.sections.iter().find(|s| s.role == "introduction").unwrap();
    assert_eq!(intro.present, 5);
    assert_eq!(intro.titles[0].text, "Introduction");
    // 300/1300 three times, 300/1200 twice
    let want = (3.0 * 300.0 / 1300.0 + 2.0 * 300.0 / 1200.0) / 5.0;
    assert!((intro.share.mean - want).abs() < 0.001, "{} vs {want}", intro.share.mean);
    assert!((intro.share.min - 300.0 / 1300.0).abs() < 0.005 && (intro.share.max - 0.25).abs() < 0.005, "{:?}", intro.share);
    // a third of the introduction is value, in that order: value → prior → contribution
    let v = intro.points.iter().find(|x| x.category == "value").unwrap();
    assert!((v.share.mean - 1.0 / 3.0).abs() < 0.01);
    assert_eq!(intro.moves.iter().map(|m| m.category.as_str()).collect::<Vec<_>>(), ["value", "prior", "contribution"]);
    assert!(intro.citation_density.mean > 0.5);
    let methods = p.sections.iter().find(|s| s.role == "methods").unwrap();
    assert_eq!(methods.titles[0].text, "Materials and Methods");
    assert_eq!(methods.subsections[0].text, "Dataset");
    assert!((methods.equation_share.mean - 2.0 / 3.0).abs() < 0.001);
    let related = p.sections.iter().find(|s| s.role == "related").unwrap();
    assert_eq!(related.present, 3);
    // order: introduction first, statements last
    assert_eq!(p.sections[0].role, "introduction");
    assert_eq!(p.sections.last().unwrap().role, "statements");
    let h = |id: &str| p.habits.iter().find(|x| x.id == id).map(|x| (x.count, x.of));
    assert_eq!(h("related_separate"), Some((3, 5)));
    assert_eq!(h("discussion_and_conclusion"), Some((5, 5)));
    assert_eq!(h("contributions_bulleted"), Some((5, 5)));
    assert_eq!(h("fig1_overview"), Some((5, 5)));
    assert_eq!(h("data_availability"), Some((5, 5)));
    assert_eq!(h("pros_cons"), Some((5, 5)));
    assert_eq!(h("limitations"), Some((5, 5)));
    assert_eq!(h("limitations_heading"), None);
    assert_eq!((p.figures.mean, p.tables.mean, p.equations.mean), (1.0, 1.0, 3.0));
    // overall shares add up to 1 per article
    let total: f64 = p.overall.iter().map(|x| x.share.mean).sum();
    assert!((total - 1.0).abs() < 0.01, "{total}");
}

#[test]
fn phrases_are_short_and_shared_never_sentences() {
    let p = build("demo-journal", &ident(), "article", "2026-11-14", "test", &five());
    assert!(!p.phrases.is_empty());
    assert!(p.phrases.iter().all(|x| (2..=5).contains(&x.text.split(' ').count()) && x.articles >= 3));
    assert!(p.phrases.iter().any(|x| x.text.contains("proposed method")), "{:?}", p.phrases);
    let best = p.phrases.iter().find(|x| x.text.contains("best of our knowledge")).unwrap();
    assert_eq!(best.situation, "contribution");
    // the statements are never mined
    assert!(!p.phrases.iter().any(|x| x.text.contains("available on request")));
}

#[test]
fn heading_labels_need_no_model() {
    let a = article(0, false);
    let l = heading_labels(&a);
    assert_eq!(l.cats.len(), a.paragraphs().len());
    assert_eq!(l.cats[0], "motivation");
    assert_eq!(l.roles.iter().filter(|r| *r == "methods").count(), 2); // Dataset inherits
}

#[test]
fn saved_and_loaded_with_a_readable_copy() {
    let d = tempfile::tempdir().unwrap();
    let p = build("demo-journal", &ident(), "article", "2026-11-14", "test", &five());
    let dir = save(d.path(), &p).unwrap();
    assert_eq!(dir, d.path().join("venues/demo-journal"));
    assert_eq!(load(d.path(), "demo-journal").unwrap(), p);
    let md = std::fs::read_to_string(dir.join("profile.md")).unwrap();
    assert!(md.contains("# 期刊寫法：Journal of Demo Imaging") && md.contains("Materials and Methods") && md.contains("doi:10.5555/demo.0"));
    // no full text is kept
    let toml_text = std::fs::read_to_string(dir.join("profile.toml")).unwrap();
    assert!(!toml_text.contains("val0 val1") && !toml_text.contains("Screening matters for patients"));
    assert!(load(d.path(), "../x").is_none());
    let mut bad = p.clone();
    bad.template = "../x".into();
    assert!(save(d.path(), &bad).is_err());
}

#[test]
fn drafting_blocks_checklist_and_skeleton() {
    let p = build("demo-journal", &ident(), "article", "2026-11-14", "test", &five());
    let sub = Submission::from_table(Some(&toml::from_str::<toml::Table>("abstract_words = 250\nrequired_statements = [\"Data availability\", \"Ethics approval\"]\nrequired_sections = \"Limitations\"\nchecked = \"2026-11-01\"\npages = \"eight\"\nhighlights = true\nfuture_key = 3\n").unwrap())).unwrap();
    assert_eq!(sub.abstract_words, Some(250));
    assert_eq!(sub.pages, None); // wrong type: left out, the rest still read
    assert_eq!(sub.required_sections, ["Limitations"]);
    assert_eq!(sub.other["future_key"], 3);
    let b = prompt_block(&p, "Introduction", Some(&sub)).unwrap();
    assert!(b.contains("not the author's voice") && b.contains("value → prior → contribution") && b.contains("\"Introduction\" 5/5"));
    assert!(b.contains("contributions_list (5/5 articles)"), "{b}");
    let m = prompt_block(&p, "3 Our Method", None).unwrap();
    assert!(m.contains("Materials and Methods") && m.contains("equations"));
    assert!(prompt_block(&p, "Appendix B", None).is_none());
    let o = outline_block(&p, Some(&sub));
    assert!(o.contains("related_separate 3/5") && o.contains("abstract ≤ 250 words") && o.contains("Ethics approval"));
    assert!(o.contains("Section \"Related Work\" (in 3/5"), "{o}");
    assert!(emphasis_block(&p).contains("pros_cons 5/5"));
    let c = audit_checklist(Some(&p), Some(&sub));
    assert!(c[0].required && c[0].label == "Data availability" && c[0].source == "submission");
    let pc = c.iter().find(|x| x.id == "pros_cons").unwrap();
    assert_eq!((pc.count, pc.of, pc.section.as_str(), pc.required), (5, 5, "discussion", false));
    assert_eq!(c.iter().find(|x| x.id == "related_separate").map(|x| (x.count, x.of)), Some((3, 5)), "3/5 counts as usual (≥ 60 %)");
    let sk = skeleton(&p);
    let titles: Vec<&str> = sk.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Introduction", "Related Work", "Materials and Methods", "Results", "Discussion", "Conclusion"]);
    assert_eq!(sk[2].subsections[0].title, "Dataset");
    assert!(summary(&p).starts_with("Introduction → Related Work → Materials and Methods"));
}

#[test]
fn identity_round_trips_and_an_odd_table_is_none() {
    let id = Identity { issn: vec!["1234-5678".into()], confirmed: "2026-11-14".into(), ..ident() };
    assert_eq!(Identity::from_table(Some(&id.to_table())), Some(id));
    let odd: toml::Table = toml::from_str("openalex = 3").unwrap();
    assert_eq!(Identity::from_table(Some(&odd)), None);
    assert_eq!(Identity::from_table(None), None);
}

#[test]
fn template_keeps_venue_and_submission_through_the_editor() {
    use crate::journal;
    let d = tempfile::tempdir().unwrap();
    let pubs = d.path();
    std::fs::create_dir_all(pubs.join("base-fmt")).unwrap();
    std::fs::write(pubs.join("base-fmt/template.toml"), "name = \"Demo Base（通用）\"\nkind = \"journal\"\n").unwrap();
    let id = journal::create(pubs, journal::Meta { name: "Journal of Demo Imaging".into(), ..Default::default() }, "base-fmt").unwrap();
    let sub: toml::Table = toml::from_str("abstract_words = 250\nchecked = \"2026-11-01\"").unwrap();
    journal::edit_meta(pubs, &id, |m| {
        m.venue = Some(ident().to_table());
        m.submission = Some(sub.clone());
    })
    .unwrap();
    // the template editor saves the description only: identity and requirements stay
    journal::update(pubs, &id, journal::Update { meta: journal::Meta { name: "Journal of Demo Imaging".into(), notes: "n".into(), ..Default::default() }, ..Default::default() }).unwrap();
    let j = journal::load(pubs, &id).unwrap();
    assert_eq!(Identity::from_table(j.meta.venue.as_ref()).unwrap().openalex, "S900001");
    assert_eq!(Submission::from_table(j.meta.submission.as_ref()).unwrap().abstract_words, Some(250));
    // a journal made from this one is another journal: no identity copied
    let other = journal::create(pubs, journal::Meta { name: "Other Demo Journal".into(), ..Default::default() }, &id).unwrap();
    assert!(journal::load(pubs, &other).unwrap().meta.venue.is_none());
    // the base (has journals on it) is hidden; journals are not
    let list = journal::list(pubs);
    assert!(list.iter().find(|j| j.id == "base-fmt").unwrap().hidden);
    assert!(!list.iter().find(|j| j.id == "other-demo-journal").unwrap().hidden);
    assert!(!list.iter().find(|j| j.id == id).unwrap().hidden, "a journal others are copied from is still a journal");
}

#[test]
fn ranking_uses_the_topic_locally() {
    let c = vec![
        ("Liver widget study".to_string(), vec!["Hepatology".to_string()], "2026-01-01".to_string()),
        ("Retinal widget segmentation".to_string(), vec!["Segmentation".to_string()], "2025-01-01".to_string()),
        ("Unrelated newest".to_string(), vec![], "2026-06-01".to_string()),
    ];
    assert_eq!(rank(&c, &["retinal segmentation".into()]), [1, 2, 0]);
    assert_eq!(rank(&c, &[]), [2, 0, 1]);
}
