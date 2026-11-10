//! A typed reference ("A. Writer, Folded widgets, J. Widgets 3 (2020)
//! 1-10.") as BibTeX fields, without the network. The common shapes:
//!
//! - IEEE: `A. Writer and B. Author, "Title," Journal, vol. 3, no. 2, pp. 1–10, 2020.`
//! - Elsevier numeric: `A. Writer, B. Author, Title, Journal 3 (2020) 1–10.`
//! - APA / Springer: `Writer, A., & Author, B. (2020). Title. Journal, 3(2), 1–10.`
//! - Vancouver: `Writer A, Author B. Title. Journal. 2020;3(2):1-10.`
//!
//! Strong signals first (DOI, URL, year, `vol./no./pp.`, `V(I):P`), then
//! the authors are what comes before the title. What cannot be told apart
//! stays in `title`; the original text is always kept in `annote`, which
//! BibTeX and citeproc styles do not print.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    /// article | inproceedings | book | misc
    pub kind: String,
    pub fields: BTreeMap<String, String>,
}

const PARTICLES: &[&str] = &["van", "von", "der", "den", "de", "del", "della", "da", "di", "du", "le", "la", "dos", "ter", "ten", "bin", "al", "el"];

fn is_year(w: &str) -> bool {
    let d: String = w.chars().take(4).collect();
    d.len() == 4 && d.chars().all(|c| c.is_ascii_digit()) && (d.starts_with("19") || d.starts_with("20")) && w.chars().skip(4).all(|c| c.is_ascii_lowercase()) && w.chars().count() <= 5
}

/// "A." "A.B." "A.-B." "J.-P." "Ch.": an initial (or several).
fn is_initials(w: &str) -> bool {
    let w = w.trim_end_matches([',', ';']);
    if !w.ends_with('.') {
        return false;
    }
    w.split(['.', '-']).filter(|p| !p.is_empty()).all(|p| {
        let mut cs = p.chars();
        cs.next().is_some_and(|c| c.is_uppercase()) && p.chars().count() <= 2
    })
}

/// "AB" in "Writer AB" (Vancouver): up to three capitals.
fn is_bare_initials(w: &str) -> bool {
    (1..=3).contains(&w.chars().count()) && w.chars().all(|c| c.is_uppercase())
}

fn capitalised(w: &str) -> bool {
    let w = w.trim_matches(|c: char| !c.is_alphanumeric());
    w.chars().next().is_some_and(|c| c.is_uppercase()) && w.chars().skip(1).all(|c| c.is_lowercase() || c == '-' || c == '\'' || c == '’' || c.is_uppercase())
}

/// Does this piece read as one person's name?
fn name_like(piece: &str, need_initial: bool) -> bool {
    let p = piece.trim();
    if p.eq_ignore_ascii_case("et al.") || p.eq_ignore_ascii_case("et al") {
        return true;
    }
    let words: Vec<&str> = p.split_whitespace().collect();
    if words.is_empty() || words.len() > 5 {
        return false;
    }
    let mut initial = false;
    for (i, w) in words.iter().enumerate() {
        if is_initials(w) || (i == words.len() - 1 && i > 0 && is_bare_initials(w)) {
            initial = true;
        } else if PARTICLES.contains(&w.to_lowercase().as_str()) || capitalised(w) {
        } else {
            return false;
        }
    }
    initial || !need_initial
}

/// "A. Writer" → "Writer, A."; "Writer AB" → "Writer, A. B."; "Writer, A." stays.
fn bib_name(n: &str) -> String {
    let n = n.trim().trim_end_matches([',', ';']).trim();
    if n.eq_ignore_ascii_case("et al.") || n.eq_ignore_ascii_case("et al") {
        return "others".into();
    }
    if n.contains(',') {
        return n.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    let words: Vec<&str> = n.split_whitespace().collect();
    match words.as_slice() {
        [] => String::new(),
        [one] => one.to_string(),
        _ => {
            // "BC." at the end (Vancouver with a full stop) reads as "BC"
            let last = words.last().map(|w| w.strip_suffix('.').filter(|b| b.chars().count() > 1 && is_bare_initials(b)).unwrap_or(w)).unwrap_or("");
            if is_bare_initials(last) && words.len() >= 2 {
                // Vancouver: family first, initials after
                let given = last.chars().map(|c| format!("{c}.")).collect::<Vec<_>>().join(" ");
                return format!("{}, {given}", words[..words.len() - 1].join(" "));
            }
            // given names (initials or words) first, then particles and the family name
            let mut k = words.len() - 1;
            while k > 0 && PARTICLES.contains(&words[k - 1].to_lowercase().as_str()) {
                k -= 1;
            }
            format!("{}, {}", words[k..].join(" "), words[..k].join(" "))
        }
    }
}

/// An author list as written → BibTeX `author`.
fn authors(s: &str) -> Vec<String> {
    let s = s.trim().trim_end_matches([',', ':', ';']).trim();
    // "and" / "&" as separators
    let s = s.replace(" & ", ", ").replace(", and ", ", ").replace(" and ", ", ").replace(",&", ",");
    let mut pieces: Vec<String> = Vec::new();
    for p in s.split([',', ';']).map(str::trim).filter(|p| !p.is_empty()) {
        // "Writer, A." (APA): the initials belong to the piece before
        if p.split_whitespace().all(|w| is_initials(w) || is_bare_initials(w)) && !pieces.is_empty() && !pieces.last().is_some_and(|l| l.contains(' ')) && !pieces.last().is_some_and(|l| l.contains(',')) {
            let last = pieces.pop().unwrap_or_default();
            pieces.push(format!("{last}, {p}"));
        } else {
            pieces.push(p.to_string());
        }
    }
    pieces.iter().map(|p| bib_name(p)).filter(|n| !n.is_empty()).collect()
}

fn find_doi(s: &str) -> Option<(usize, usize, String)> {
    let b = s.as_bytes();
    let mut from = 0;
    while let Some(i) = s[from..].find("10.").map(|k| from + k) {
        let mut j = i + 3;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        if j - (i + 3) >= 4 && b.get(j) == Some(&b'/') {
            let mut e = j + 1;
            while e < b.len() && !b[e].is_ascii_whitespace() {
                e += 1;
            }
            let doi = s[i..e].trim_end_matches(['.', ',', ';', ')', ']']).to_string();
            // the "doi:" / URL in front goes too
            let mut start = i;
            for p in ["https://doi.org/", "http://doi.org/", "https://dx.doi.org/", "http://dx.doi.org/", "doi.org/", "doi: ", "doi:", "DOI: ", "DOI:", "DOI ", "doi "] {
                if s[..i].ends_with(p) {
                    start = i - p.len();
                    break;
                }
            }
            return Some((start, i + doi.len(), doi));
        }
        from = i + 3;
    }
    None
}

fn find_url(s: &str) -> Option<(usize, usize, String)> {
    let i = s.find("https://").or_else(|| s.find("http://"))?;
    let e = s[i..].find(char::is_whitespace).map(|k| i + k).unwrap_or(s.len());
    let url = s[i..e].trim_end_matches(['.', ',', ';', ')']).to_string();
    let mut start = i;
    for p in ["Available: ", "Available at: ", "URL: ", "[Online]. Available: "] {
        if s[..i].ends_with(p) {
            start = i - p.len();
        }
    }
    Some((start, i + url.len(), url))
}

fn cut(s: &str, a: usize, b: usize) -> String {
    format!("{} {}", s[..a].trim_end(), s[b..].trim_start()).trim().to_string()
}

fn dash_pages(p: &str) -> String {
    p.trim().replace(['–', '—', '‐', '−'], "-").replace("--", "-").replace('-', "--")
}

/// The venue part ("Journal of Widgets 3 (2020) 1-10", "J. Widgets, vol. 3,
/// no. 2, pp. 1–10, 2020", "Journal, 3(2), 1–10", "J Widgets. 2020;3(2):1-10")
/// → venue name and the numbers found.
fn venue_numbers(rest: &str, f: &mut BTreeMap<String, String>) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut venue_done = false;
    let toks: Vec<&str> = rest.split_whitespace().collect();
    let mut i = 0;
    let set = |f: &mut BTreeMap<String, String>, k: &str, v: &str| {
        let v = v.trim().trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':' | '(' | ')'));
        if !v.is_empty() && !f.contains_key(k) {
            f.insert(k.to_string(), v.to_string());
        }
    };
    while i < toks.len() {
        let t = toks[i];
        let low = t.to_lowercase();
        let next = toks.get(i + 1).copied().unwrap_or("");
        if matches!(low.as_str(), "vol." | "vol" | "volume" | "bd.") {
            set(f, "volume", next);
            venue_done = true;
            i += 2;
            continue;
        }
        if matches!(low.as_str(), "no." | "no" | "issue" | "nr." | "iss.") {
            set(f, "number", next);
            venue_done = true;
            i += 2;
            continue;
        }
        if matches!(low.as_str(), "pp." | "p." | "pages" | "pp" | "s.") {
            set(f, "pages", &dash_pages(next.trim_end_matches([',', '.', ';'])));
            venue_done = true;
            i += 2;
            continue;
        }
        let bare = t.trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':'));
        // 2020;3(2):1-10  |  3(2):1-10  |  3(2), 1–10  |  2020;3:1-10
        if bare.chars().next().is_some_and(|c| c.is_ascii_digit()) && (bare.contains('(') || bare.contains(';') || bare.contains(':')) {
            let mut s = bare.to_string();
            if let Some((y, r)) = s.split_once(';') {
                if is_year(y.trim()) {
                    set(f, "year", y);
                }
                s = r.to_string();
            }
            let (head, pages) = match s.split_once(':') {
                Some((h, p)) => (h.to_string(), Some(p.to_string())),
                None => (s.clone(), None),
            };
            let (vol, num) = match head.split_once('(') {
                Some((v, n)) => (v.to_string(), Some(n.trim_end_matches(')').to_string())),
                None => (head.clone(), None),
            };
            if !vol.is_empty() && vol.chars().all(|c| c.is_ascii_digit()) {
                if is_year(&vol) && num.is_none() && pages.is_none() {
                    set(f, "year", &vol);
                } else {
                    set(f, "volume", &vol);
                }
            }
            if let Some(n) = num {
                if is_year(&n) {
                    set(f, "year", &n);
                } else {
                    set(f, "number", &n);
                }
            }
            match pages {
                Some(p) if !p.is_empty() => set(f, "pages", &dash_pages(&p)),
                // "3(2), 1–10"
                _ if t.ends_with(',') && next.chars().next().is_some_and(|c| c.is_ascii_digit()) && !is_year(next.trim_end_matches(['.', ','])) => {
                    set(f, "pages", &dash_pages(next.trim_end_matches(['.', ',', ';'])));
                    i += 1;
                }
                _ => {}
            }
            venue_done = true;
            i += 1;
            continue;
        }
        // (2020)
        if bare.starts_with('(') && is_year(bare.trim_matches(['(', ')'])) {
            set(f, "year", bare.trim_matches(['(', ')']));
            venue_done = true;
            i += 1;
            continue;
        }
        if is_year(bare) {
            set(f, "year", bare);
            venue_done = true;
            i += 1;
            continue;
        }
        // 1-10 / 1–10 / e123
        if bare.chars().next().is_some_and(|c| c.is_ascii_digit()) && bare.contains(['-', '–', '—']) {
            set(f, "pages", &dash_pages(bare));
            venue_done = true;
            i += 1;
            continue;
        }
        // a plain number: the volume after the venue name ("Journal 3 (2020)")
        if bare.chars().all(|c| c.is_ascii_digit()) && !bare.is_empty() {
            if !f.contains_key("volume") {
                set(f, "volume", bare);
            } else if !f.contains_key("pages") {
                set(f, "pages", bare);
            }
            venue_done = true;
            i += 1;
            continue;
        }
        if !venue_done {
            words.push(t.to_string());
            // "Journal," ends the name
            if t.ends_with(',') {
                venue_done = true;
            }
        }
        i += 1;
    }
    let v = words.join(" ");
    let v = v.trim().trim_end_matches([',', '.', ';', ':']).trim();
    let v = v.strip_prefix("In: ").or_else(|| v.strip_prefix("In ")).or_else(|| v.strip_prefix("in ")).unwrap_or(v);
    v.trim().to_string()
}

/// End of the first sentence (". ", "? ", "! ") that is not an initial.
fn sentence_end(s: &str) -> Option<usize> {
    let cs: Vec<(usize, char)> = s.char_indices().collect();
    for (k, &(i, c)) in cs.iter().enumerate() {
        if !matches!(c, '.' | '?' | '!') {
            continue;
        }
        let at_end = k + 1 == cs.len();
        if !at_end && !cs[k + 1].1.is_whitespace() {
            continue;
        }
        if c == '.' {
            // "A." "Proc." "J." are not the end
            let word: String = s[..i].rsplit(char::is_whitespace).next().unwrap_or("").to_string();
            if word.chars().count() <= 1 && word.chars().all(|c| c.is_uppercase()) && !word.is_empty() {
                continue;
            }
        }
        return Some(i);
    }
    None
}

fn tidy(s: &str) -> String {
    s.trim().trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':' | ' ')).trim().to_string()
}

const PROC: &[&str] = &["proceedings", "proc.", "conference", "conf.", "symposium", "workshop", "congress"];
const PUBLISHERS: &[&str] = &["press", "publishers", "publishing", "springer", "wiley", "elsevier", "crc", "routledge", "academic", "verlag", "sage"];

/// Parse one reference as written.
pub fn parse(original: &str) -> Parsed {
    let mut f: BTreeMap<String, String> = BTreeMap::new();
    let mut s = original.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some((a, b, doi)) = find_doi(&s) {
        f.insert("doi".into(), doi);
        s = cut(&s, a, b);
    }
    if let Some((a, b, url)) = find_url(&s) {
        if !f.contains_key("doi") {
            f.insert("url".into(), url);
        }
        s = cut(&s, a, b);
    }
    for p in ["arXiv:", "arXiv preprint arXiv:"] {
        if let Some(i) = s.find(p) {
            let e = s[i + p.len()..].find(char::is_whitespace).map(|k| i + p.len() + k).unwrap_or(s.len());
            f.insert("eprint".into(), tidy(&s[i + p.len()..e]));
            f.insert("archiveprefix".into(), "arXiv".into());
        }
    }
    let s = s.trim().to_string();

    let (names, title, rest): (Vec<String>, String, String);
    // 1. a quoted title (IEEE)
    let quote = s.char_indices().find(|(_, c)| matches!(c, '“' | '"' | '«'));
    let apa = find_apa_year(&s);
    if let Some((qi, qc)) = quote.filter(|(qi, _)| *qi > 0) {
        let close = match qc { '“' => '”', '«' => '»', _ => '"' };
        let after = &s[qi + qc.len_utf8()..];
        let (t, r) = match after.find(close) {
            Some(e) => (&after[..e], &after[e + close.len_utf8()..]),
            None => (after, ""),
        };
        names = authors(&s[..qi]);
        title = tidy(t);
        rest = r.trim().trim_start_matches([',', '.']).trim().to_string();
    } else if let Some((ya, yb, year)) = apa {
        // 2. Authors (2020). Title. Venue …
        f.insert("year".into(), year);
        names = authors(&s[..ya]);
        let after = s[yb..].trim_start_matches(['.', ',', ' ']).trim();
        match sentence_end(after) {
            Some(e) => {
                title = tidy(&after[..e]);
                rest = after[e + 1..].trim().to_string();
            }
            None => {
                title = tidy(after);
                rest = String::new();
            }
        }
    } else if let Some((a, t, r)) = vancouver(&s) {
        // 3. Writer A, Author B. Title. Venue. 2020;3(2):1-10.
        names = authors(&a);
        title = t;
        rest = r;
    } else {
        // 4. comma separated: names, then the title, then the venue
        let pieces: Vec<&str> = s.split(", ").collect();
        let mut k = 0;
        while k < pieces.len() {
            let p = pieces[k].trim().trim_start_matches("and ").trim_start_matches("& ");
            // "A. Writer and B. Author" in one piece
            let all_names = p.split(" and ").all(|x| name_like(x, true));
            if all_names && k + 1 < pieces.len() {
                k += 1;
            } else {
                break;
            }
        }
        if k > 0 {
            names = authors(&pieces[..k].join(", "));
            let rest_all = pieces[k..].join(", ");
            // the title ends at the first ". " or ", " before the venue
            let cut_at = match (sentence_end(&rest_all), pieces.get(k).map(|p| p.len())) {
                (Some(e), Some(c)) if e < c => e,
                (_, Some(c)) => c.min(rest_all.len()),
                _ => rest_all.len(),
            };
            title = tidy(&rest_all[..cut_at]);
            rest = rest_all[cut_at..].trim_start_matches(['.', ',', ' ']).to_string();
        } else {
            names = vec![];
            match sentence_end(&s) {
                Some(e) => {
                    title = tidy(&s[..e]);
                    rest = s[e + 1..].trim().to_string();
                }
                None => {
                    title = tidy(&s);
                    rest = String::new();
                }
            }
        }
    }
    let venue = venue_numbers(&rest, &mut f);
    if !names.is_empty() {
        f.insert("author".into(), names.join(" and "));
    }
    if !title.is_empty() {
        f.insert("title".into(), title);
    }
    let low = venue.to_lowercase();
    let kind = if !venue.is_empty() && PROC.iter().any(|p| low.contains(p)) {
        f.insert("booktitle".into(), venue);
        "inproceedings"
    } else if !venue.is_empty() && !f.contains_key("volume") && !f.contains_key("pages") && PUBLISHERS.iter().any(|p| low.split_whitespace().any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == *p)) {
        f.insert("publisher".into(), venue);
        "book"
    } else if !venue.is_empty() {
        f.insert("journal".into(), venue);
        "article"
    } else {
        "misc"
    };
    f.insert("annote".into(), original.split_whitespace().collect::<Vec<_>>().join(" "));
    Parsed { kind: kind.into(), fields: f }
}

/// "(2020)." or "(2020a)" or "(n.d.)" after the authors: (start, end, year).
fn find_apa_year(s: &str) -> Option<(usize, usize, String)> {
    let mut from = 0;
    while let Some(i) = s[from..].find('(').map(|k| from + k) {
        let Some(e) = s[i..].find(')').map(|k| i + k) else { break };
        let inner = s[i + 1..e].trim();
        let inner_year = inner.split(',').next().unwrap_or("").trim();
        if is_year(inner_year) && i > 0 && name_like_list(&s[..i]) {
            return Some((i, e + 1, inner_year.chars().take(4).collect()));
        }
        from = e;
    }
    None
}

/// Every piece of an author list is a name (APA "Writer, A., & Author, B.").
fn name_like_list(s: &str) -> bool {
    let ns = authors(s);
    !ns.is_empty() && ns.iter().all(|n| n == "others" || n.split_once(", ").is_some_and(|(fam, giv)| !fam.is_empty() && (giv.split_whitespace().all(|w| is_initials(w) || capitalised(w)))))
}

/// Vancouver: the first sentence is "Family AB, Family C" (initials
/// without dots), then the title sentence, then the rest.
fn vancouver(s: &str) -> Option<(String, String, String)> {
    let e = s.find(". ")?;
    let head = &s[..e];
    let ok = head.split(',').map(str::trim).filter(|p| !p.is_empty()).all(|p| {
        let w: Vec<&str> = p.split_whitespace().collect();
        (w.len() >= 2 && is_bare_initials(w[w.len() - 1]) && w[..w.len() - 1].iter().all(|x| capitalised(x) || PARTICLES.contains(&x.to_lowercase().as_str()))) || p == "et al"
    });
    if !ok {
        return None;
    }
    let after = &s[e + 2..];
    let t = sentence_end(after)?;
    Some((head.to_string(), tidy(&after[..t]), after[t + 1..].trim().to_string()))
}

/// The entry as BibTeX text with key `key`.
pub fn entry(key: &str, p: &Parsed) -> String {
    let order = ["author", "title", "journal", "booktitle", "publisher", "year", "volume", "number", "pages", "doi", "url", "eprint", "archiveprefix", "annote"];
    let mut out = format!("@{}{{{key},\n", p.kind);
    let mut keys: Vec<&String> = p.fields.keys().collect();
    keys.sort_by_key(|k| order.iter().position(|o| o == k).unwrap_or(99));
    for k in keys {
        // titles and venues as the author typed them: braced, so no style changes their case
        let v = escape(&p.fields[k]);
        if matches!(k.as_str(), "title" | "booktitle" | "journal" | "publisher") {
            out += &format!("  {k} = {{{{{v}}}}},\n");
        } else {
            out += &format!("  {k} = {{{v}}},\n");
        }
    }
    out + "}\n\n"
}

/// Special characters escaped, braces kept balanced.
pub fn escape(s: &str) -> String {
    let mut o = String::new();
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '&' | '%' | '$' | '#' | '_' => {
                o.push('\\');
                o.push(c);
            }
            '{' => {
                depth += 1;
                o.push(c);
            }
            '}' if depth > 0 => {
                depth -= 1;
                o.push(c);
            }
            '}' | '\\' => {}
            _ => o.push(c),
        }
    }
    o + &"}".repeat(depth.max(0) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get<'a>(p: &'a Parsed, k: &str) -> &'a str {
        p.fields.get(k).map(|s| s.as_str()).unwrap_or("")
    }

    #[test]
    fn elsevier_numeric() {
        let p = parse("A. Writer, Folded widgets, Journal of Widgets 3 (2020) 1-10.");
        assert_eq!(p.kind, "article");
        assert_eq!((get(&p, "author"), get(&p, "title"), get(&p, "journal")), ("Writer, A.", "Folded widgets", "Journal of Widgets"));
        assert_eq!((get(&p, "year"), get(&p, "volume"), get(&p, "pages")), ("2020", "3", "1--10"));
        let p = parse("A.B. Writer, C. van der Berg, D. Person, Loads on widgets, Proceedings of Widget Day (2022) 21-30.");
        assert_eq!(p.kind, "inproceedings");
        assert_eq!(get(&p, "author"), "Writer, A.B. and van der Berg, C. and Person, D.");
        assert_eq!((get(&p, "title"), get(&p, "booktitle"), get(&p, "year"), get(&p, "pages")), ("Loads on widgets", "Proceedings of Widget Day", "2022", "21--30"));
    }

    #[test]
    fn ieee() {
        let p = parse("A. Writer, B. Author, and C. Person, “Folded widgets under load,” Journal of Widgets, vol. 3, no. 2, pp. 1–10, 2020, doi: 10.1234/jw.2020.003.");
        assert_eq!(p.kind, "article");
        assert_eq!(get(&p, "author"), "Writer, A. and Author, B. and Person, C.");
        assert_eq!((get(&p, "title"), get(&p, "journal")), ("Folded widgets under load", "Journal of Widgets"));
        assert_eq!((get(&p, "volume"), get(&p, "number"), get(&p, "pages"), get(&p, "year"), get(&p, "doi")), ("3", "2", "1--10", "2020", "10.1234/jw.2020.003"));
        let p = parse("A. Writer and B. Author, \"Widget strength,\" in Proc. Widget Conf., 2021, pp. 11-20.");
        assert_eq!(p.kind, "inproceedings");
        assert_eq!((get(&p, "author"), get(&p, "booktitle"), get(&p, "year"), get(&p, "pages")), ("Writer, A. and Author, B.", "Proc. Widget Conf", "2021", "11--20"));
    }

    #[test]
    fn apa_and_springer() {
        let p = parse("Writer, A., & Author, B. (2020). Folded widgets under load. Journal of Widgets, 3(2), 1–10. https://doi.org/10.1234/jw.3");
        assert_eq!(p.kind, "article");
        assert_eq!(get(&p, "author"), "Writer, A. and Author, B.");
        assert_eq!((get(&p, "title"), get(&p, "journal"), get(&p, "year")), ("Folded widgets under load", "Journal of Widgets", "2020"));
        assert_eq!((get(&p, "volume"), get(&p, "number"), get(&p, "pages"), get(&p, "doi")), ("3", "2", "1--10", "10.1234/jw.3"));
        let p = parse("Writer A, Author B (2021) Widget strength revisited. Widget Lett 5(1):11–20");
        assert_eq!(get(&p, "author"), "Writer, A. and Author, B.");
        assert_eq!((get(&p, "title"), get(&p, "journal"), get(&p, "year"), get(&p, "volume"), get(&p, "number"), get(&p, "pages")), ("Widget strength revisited", "Widget Lett", "2021", "5", "1", "11--20"));
    }

    #[test]
    fn vancouver_style() {
        let p = parse("Writer A, Author BC. Folded widgets under load. J Widgets. 2020;3(2):1-10.");
        assert_eq!(get(&p, "author"), "Writer, A. and Author, B. C.");
        assert_eq!((get(&p, "title"), get(&p, "journal"), get(&p, "year"), get(&p, "volume"), get(&p, "number"), get(&p, "pages")), ("Folded widgets under load", "J Widgets", "2020", "3", "2", "1--10"));
    }

    #[test]
    fn books_and_leftovers() {
        let p = parse("D. Name, The Widget Book, Widget Press, 2019.");
        assert_eq!(p.kind, "book");
        assert_eq!((get(&p, "title"), get(&p, "publisher"), get(&p, "year")), ("The Widget Book", "Widget Press", "2019"));
        let p = parse("Some report nobody can parse");
        assert_eq!(p.kind, "misc");
        assert_eq!(get(&p, "title"), "Some report nobody can parse");
        assert_eq!(get(&p, "annote"), "Some report nobody can parse");
    }

    #[test]
    fn bibtex_text() {
        let p = parse("A. Writer, R&D of widgets, J. Widgets 3 (2020) 1-10.");
        let e = entry("ref1", &p);
        assert!(e.starts_with("@article{ref1,\n  author = {Writer, A.},\n  title = {{R\\&D of widgets}},"), "{e}");
        assert!(e.contains("annote = {A. Writer, R\\&D of widgets, J. Widgets 3 (2020) 1-10.}"), "{e}");
    }
}
