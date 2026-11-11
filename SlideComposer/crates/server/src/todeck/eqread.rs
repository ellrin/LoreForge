//! Equations LoreReader could not read reliably from the PDF's text (LaTeX that KaTeX
//! refuses or whose brackets do not pair, `lore_math::check::check_reading`): a vision
//! engine reads the equation's crop (task `sc-read-equation`, this crate's `tasks.toml`);
//! its answer is checked the same way, else the slide shows the crop as a picture. A
//! slide never shows LaTeX source as text.

use lore_llm::{Image, LlmRequest};
use serde_json::{Value, json};

use super::source::{Eq, Img, Material};
use crate::llm::Llm;

pub const TASK: &str = "sc-read-equation";

const SYSTEM: &str = "You transcribe one display equation from an image of a research paper into LaTeX that KaTeX can typeset. \
Write exactly what the image shows: \\frac for stacked fractions, \\sim for 'distributed as', \\tilde / \\hat / \\bar accents, x' for primes, \
\\mathrm{…} for words in subscripts (\\mathrm{real}), \\mathcal only for calligraphic capitals, \\mathbb for blackboard letters, \\boldsymbol for bold italic. \
Several rows go in \\begin{aligned} … \\end{aligned} with & before the relation. Leave out the equation number. \
Reply with JSON {\"latex\": \"…\", \"readable\": true}; readable = false when the image is not legible.";

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": { "latex": { "type": "string" }, "readable": { "type": "boolean" } },
        "required": ["latex"]
    })
}

/// LaTeX of the PDF's text that should not reach a slide as it is.
pub fn unreliable(e: &Eq) -> bool {
    e.read == "text" && lore_math::check::check_reading(&e.latex).is_err()
}

/// A model's LaTeX without math delimiters or an equation number.
fn clean(s: &str) -> String {
    let mut t = s.trim();
    for (a, b) in [("$$", "$$"), ("\\[", "\\]"), ("$", "$")] {
        if let Some(x) = t.strip_prefix(a).and_then(|x| x.strip_suffix(b)) {
            t = x.trim();
        }
    }
    let t = regex::Regex::new(r"\\(tag|label)\{[^}]*\}").unwrap().replace_all(t, "");
    t.trim().to_string()
}

/// Read each unreliable equation of `m` from its crop (`crop(e)`, `None` when there is no
/// page or no renderer). Returns notes for the job (one line each).
pub async fn read_equations(llm: &Llm, m: &mut Material, crop: &(dyn Fn(&Eq) -> Option<Img> + Send + Sync)) -> Vec<String> {
    let mut notes = vec![];
    for i in 0..m.equations.len() {
        if !unreliable(&m.equations[i]) {
            continue;
        }
        let id = m.equations[i].id.clone();
        let Some(img) = crop(&m.equations[i]) else {
            // no picture either: the slides leave it out
            m.equations[i].read = "picture".into();
            notes.push(format!("{id}：PDF 的文字讀不出這條式子，也沒有頁面可以截圖，投影片不放"));
            continue;
        };
        let user = format!("The equation {} of the paper. Its text as extracted (unreliable, for orientation only): {}", m.equations[i].label.clone().unwrap_or_default(), m.equations[i].latex);
        let req = LlmRequest::prompt(SYSTEM, user).json(schema()).max_tokens(2000).image(Image::from_bytes("image/png", &img.bytes));
        let read = match llm.complete_json(TASK, req, None).await {
            Ok(a) if a.value["readable"] != false => {
                let latex = clean(a.value["latex"].as_str().unwrap_or(""));
                match lore_math::check::check_reading(&latex) {
                    Ok(()) => Some(latex),
                    Err(why) => {
                        notes.push(format!("{id}：模型讀出的式子無法排版（{why}），改放截圖"));
                        None
                    }
                }
            }
            Ok(_) => {
                notes.push(format!("{id}：模型讀不清這條式子，改放截圖"));
                None
            }
            Err((why, _)) => {
                notes.push(format!("{id}：沒有能看圖的模型（{why}），改放截圖"));
                None
            }
        };
        let e = &mut m.equations[i];
        match read {
            Some(latex) => {
                e.latex = latex;
                e.read = "vision".into();
            }
            None => {
                e.read = "picture".into();
                e.image = Some(img);
            }
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use lore_llm::MockProvider;
    use lore_llm::router::MockFactory;
    use std::sync::Arc;

    fn eq(id: &str, latex: &str) -> Eq {
        Eq { id: id.into(), latex: latex.into(), label: Some("(1)".into()), page: Some(3), bbox: Some([0.2, 0.3, 0.8, 0.4]), read: "text".into(), image: None }
    }

    fn img() -> Img {
        Img { bytes: vec![1, 2, 3], file: String::new(), ext: "png".into(), w: 10, h: 4 }
    }

    #[tokio::test]
    async fn broken_text_is_read_from_the_crop_else_shown_as_a_picture() {
        let dir = tempfile::tempdir().unwrap();
        let replies = vec![
            json!({ "latex": "$$\\frac{1}{2}\\mathbb{E}_{\\tilde{x}\\sim P_{\\mathrm{syn}}}[D(\\tilde{x})^{2}] \\tag{1}$$", "readable": true }).to_string(),
            json!({ "latex": "\\frac{1}{2", "readable": true }).to_string(),
        ];
        let vision = Arc::new(MockProvider::new(replies));
        let llm = Llm::with_factory(lore_home::Home::at(dir.path()), MockFactory::default().with("claude:selected", vision.clone()));
        let mut m = Material {
            equations: vec![
                eq("eq:1", "1 2\\mathbb{E}_{\\mathcal{x}\u{303}~P}"),
                eq("eq:2", "\\mathcal{L} = a + b"),
                eq("eq:3", "W(\\mathbb{P}_{r}, \\mathbb{E}[f]"),
            ],
            ..Default::default()
        };
        let notes = read_equations(&llm, &mut m, &|_| Some(img())).await;
        let e = &m.equations;
        assert_eq!((e[0].read.as_str(), e[0].latex.as_str()), ("vision", "\\frac{1}{2}\\mathbb{E}_{\\tilde{x}\\sim P_{\\mathrm{syn}}}[D(\\tilde{x})^{2}]"));
        assert!(e[0].image.is_none());
        assert_eq!(e[1].read, "text", "a reliable reading is kept");
        assert_eq!(e[2].read, "picture", "the model's answer does not typeset");
        assert!(e[2].image.is_some() && !e[2].usable());
        assert_eq!(notes.len(), 1, "{notes:?}");
        let seen = vision.seen.lock().unwrap();
        assert_eq!(seen.len(), 2, "only the unreliable ones are sent");
        assert!(seen[0].has_images());

        // no engine at all: the crop is the picture
        let none = Llm::none(lore_home::Home::at(dir.path()));
        let mut m2 = Material { equations: vec![eq("eq:1", "x_{a}_{b}")], ..Default::default() };
        read_equations(&none, &mut m2, &|_| Some(img())).await;
        assert_eq!(m2.equations[0].read, "picture");
        // nor a page to crop: left out
        let mut m3 = Material { equations: vec![eq("eq:1", "x_{a}_{b}")], ..Default::default() };
        read_equations(&none, &mut m3, &|_| None).await;
        assert!(!m3.equations[0].usable() && m3.equations[0].image.is_none());
    }
}
