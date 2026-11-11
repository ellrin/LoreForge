//! A KaTeX-style check of one equation's LaTeX without a browser: what KaTeX refuses to
//! typeset (and pandoc turns into garbage OMML) is refused here, so a program that read
//! an equation out of a PDF can tell a usable reading from a broken one.
//!
//! Refused: characters KaTeX cannot place (combining marks, controls, private use,
//! Unicode math letters), unbalanced `{}` / `\left…\right` / `\begin…\end`, unknown
//! commands and environments, a command without its argument, an empty or double
//! sub/superscript, `&` outside an alignment environment.

/// Commands KaTeX typesets (the common math subset; registry macros are expanded before).
const COMMANDS: &[&str] = &[
    // letters
    "alpha", "beta", "gamma", "delta", "epsilon", "varepsilon", "zeta", "eta", "theta", "vartheta", "iota", "kappa", "varkappa", "lambda", "mu", "nu",
    "xi", "pi", "varpi", "rho", "varrho", "sigma", "varsigma", "tau", "upsilon", "phi", "varphi", "chi", "psi", "omega", "Gamma", "Delta", "Theta", "Lambda",
    "Xi", "Pi", "Sigma", "Upsilon", "Phi", "Psi", "Omega", "varGamma", "varDelta", "varTheta", "varLambda", "varXi", "varPi", "varSigma", "varUpsilon",
    "varPhi", "varPsi", "varOmega", "ell", "hbar", "imath", "jmath", "aleph", "wp", "Re", "Im", "partial", "nabla", "infty", "emptyset", "varnothing",
    // fonts and text
    "mathrm", "mathit", "mathbf", "mathsf", "mathtt", "mathcal", "mathscr", "mathbb", "mathfrak", "boldsymbol", "bm", "mathup", "text", "textrm", "textit",
    "textbf", "textsf", "texttt", "mbox", "operatorname", "rm", "bf", "it", "cal",
    // accents
    "hat", "widehat", "tilde", "widetilde", "bar", "overline", "underline", "vec", "dot", "ddot", "check", "breve", "acute", "grave", "mathring",
    "overrightarrow", "overleftarrow", "overbrace", "underbrace",
    // structures
    "frac", "dfrac", "tfrac", "cfrac", "binom", "dbinom", "tbinom", "sqrt", "left", "right", "middle", "big", "Big", "bigg", "Bigg", "bigl", "bigr", "Bigl",
    "Bigr", "biggl", "biggr", "Biggl", "Biggr", "begin", "end", "limits", "nolimits", "displaystyle", "textstyle", "scriptstyle", "stackrel", "overset",
    "underset", "substack", "tag", "notag", "nonumber", "label", "boxed", "color", "phantom", "hphantom", "vphantom",
    // spaces
    "quad", "qquad", "enspace", "thinspace", "medspace", "thickspace", ",", ";", ":", "!", " ", "hspace", "mspace", "kern", "mkern", "newline",
    // operators and relations
    "sum", "prod", "coprod", "int", "iint", "iiint", "oint", "bigcup", "bigcap", "bigvee", "bigwedge", "bigoplus", "bigotimes", "bigodot", "bigsqcup", "log",
    "ln", "lg", "exp", "sin", "cos", "tan", "cot", "sec", "csc", "sinh", "cosh", "tanh", "arcsin", "arccos", "arctan", "max", "min", "sup", "inf", "lim",
    "liminf", "limsup", "arg", "det", "dim", "ker", "deg", "gcd", "hom", "Pr", "mod", "bmod", "pmod", "argmax", "argmin", "cdot", "cdots", "ldots",
    "dots", "vdots", "ddots", "times", "div", "pm", "mp", "ast", "star", "circ", "bullet", "oplus", "ominus", "otimes", "oslash", "odot", "cup", "cap",
    "setminus", "wedge", "vee", "land", "lor", "neg", "lnot", "sqcup", "sqcap", "uplus", "dagger", "ddagger", "leq", "le", "geq", "ge", "neq", "ne", "approx",
    "sim", "simeq", "cong", "equiv", "propto", "ll", "gg", "prec", "succ", "preceq", "succeq", "in", "notin", "ni", "subset", "supset", "subseteq",
    "supseteq", "mid", "parallel", "perp", "top", "bot", "models", "vdash", "dashv", "coloneqq", "doteq", "asymp", "leqslant", "geqslant", "lesssim",
    "gtrsim", "to", "gets", "rightarrow", "leftarrow", "Rightarrow", "Leftarrow", "leftrightarrow", "Leftrightarrow", "longrightarrow", "longleftarrow",
    "Longrightarrow", "Longleftarrow", "mapsto", "longmapsto", "uparrow", "downarrow", "Uparrow", "Downarrow", "implies", "iff", "forall", "exists",
    "nexists", "prime", "angle", "triangle", "square", "Box", "diamond", "surd", "colon", "xrightarrow", "xleftarrow",
    // delimiters
    "langle", "rangle", "lvert", "rvert", "lVert", "rVert", "lfloor", "rfloor", "lceil", "rceil", "vert", "Vert", "|", "{", "}", "lbrace", "rbrace",
    "lbrack", "rbrack", "backslash", "\\", "_", "%", "#", "&", "$",
];

const ENVS: &[&str] = &[
    "aligned", "align", "align*", "gathered", "gather", "gather*", "split", "cases", "dcases", "matrix", "pmatrix", "bmatrix", "Bmatrix", "vmatrix",
    "Vmatrix", "smallmatrix", "array", "alignedat", "equation", "equation*", "multline", "subarray",
];

/// Commands that take a required argument: how many.
fn arity(cmd: &str) -> usize {
    match cmd {
        "frac" | "dfrac" | "tfrac" | "cfrac" | "binom" | "dbinom" | "tbinom" | "stackrel" | "overset" | "underset" | "color" => 2,
        "sqrt" | "mathrm" | "mathit" | "mathbf" | "mathsf" | "mathtt" | "mathcal" | "mathscr" | "mathbb" | "mathfrak" | "boldsymbol" | "bm" | "mathup"
        | "text" | "textrm" | "textit" | "textbf" | "textsf" | "texttt" | "mbox" | "operatorname" | "hat" | "widehat" | "tilde" | "widetilde" | "bar"
        | "overline" | "underline" | "vec" | "dot" | "ddot" | "check" | "breve" | "acute" | "grave" | "mathring" | "overrightarrow" | "overleftarrow"
        | "overbrace" | "underbrace" | "boxed" | "phantom" | "hphantom" | "vphantom" | "substack" | "tag" | "label" | "hspace" | "mspace" | "kern"
        | "mkern" | "xrightarrow" | "xleftarrow" | "pmod" => 1,
        _ => 0,
    }
}

const DELIMS: &[&str] = &[
    "(", ")", "[", "]", "|", ".", "/", "<", ">", "\\{", "\\}", "\\|", "\\langle", "\\rangle", "\\lvert", "\\rvert", "\\lVert", "\\rVert", "\\lfloor",
    "\\rfloor", "\\lceil", "\\rceil", "\\vert", "\\Vert", "\\lbrace", "\\rbrace", "\\lbrack", "\\rbrack", "\\uparrow", "\\downarrow", "\\backslash",
];

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Cmd(String),
    Open,
    Close,
    Sub,
    Sup,
    Prime,
    Amp,
    Space,
    Ch(char),
}

fn tokens(s: &str) -> Result<Vec<Tok>, String> {
    let cs: Vec<char> = s.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        i += 1;
        let u = c as u32;
        if matches!(u, 0x0300..=0x036F | 0x20D0..=0x20FF) {
            return Err(format!("combining mark U+{u:04X}"));
        }
        if (c.is_control() && !c.is_whitespace()) || matches!(u, 0xE000..=0xF8FF | 0xFFFD) || (0x1D400..=0x1D7FF).contains(&u) {
            return Err(format!("character U+{u:04X}"));
        }
        match c {
            '\\' => {
                let Some(&n) = cs.get(i) else { return Err("a lone backslash".into()) };
                if n.is_ascii_alphabetic() {
                    let start = i;
                    while i < cs.len() && cs[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let mut name: String = cs[start..i].iter().collect();
                    if cs.get(i) == Some(&'*') && (name == "operatorname") {
                        name.push('*');
                        i += 1;
                    }
                    out.push(Tok::Cmd(name));
                } else {
                    i += 1;
                    out.push(Tok::Cmd(n.to_string()));
                }
            }
            '{' => out.push(Tok::Open),
            '}' => out.push(Tok::Close),
            '_' => out.push(Tok::Sub),
            '^' => out.push(Tok::Sup),
            '\'' => out.push(Tok::Prime),
            '&' => out.push(Tok::Amp),
            '~' => out.push(Tok::Space),
            c if c.is_whitespace() => out.push(Tok::Space),
            c => out.push(Tok::Ch(c)),
        }
    }
    Ok(out)
}

/// The text of a `{…}` group starting at `toks[i]` (an `Open`) and the index after it.
fn group_text(toks: &[Tok], i: usize) -> Option<(String, usize)> {
    if toks.get(i) != Some(&Tok::Open) {
        return None;
    }
    let mut s = String::new();
    let mut j = i + 1;
    while j < toks.len() {
        match &toks[j] {
            Tok::Close => return Some((s, j + 1)),
            Tok::Ch(c) => s.push(*c),
            Tok::Cmd(c) => s.push_str(c),
            _ => {}
        }
        j += 1;
    }
    None
}

/// Script state of the atom being built.
#[derive(Default, Clone, Copy)]
struct Scripts {
    sub: bool,
    sup: bool,
    prime: bool,
}

/// `Ok` when KaTeX (display mode, strict) can typeset `latex`.
pub fn check(latex: &str) -> Result<(), String> {
    let t = latex.trim();
    if t.is_empty() {
        return Err("empty".into());
    }
    let toks = tokens(t)?;
    let next_solid = |k: usize| (k..toks.len()).find(|&j| toks[j] != Tok::Space);
    // (scripts of the current atom) per open group; environments; \left depth per group
    let mut groups: Vec<Scripts> = vec![Scripts::default()];
    let mut envs: Vec<String> = vec![];
    let mut lefts: Vec<usize> = vec![0];
    let mut i = 0;
    while i < toks.len() {
        let tok = toks[i].clone();
        i += 1;
        let cur = groups.last_mut().unwrap();
        match tok {
            Tok::Space => {}
            Tok::Open => {
                *cur = Scripts::default();
                groups.push(Scripts::default());
                lefts.push(0);
            }
            Tok::Close => {
                if groups.len() == 1 {
                    return Err("unbalanced }".into());
                }
                groups.pop();
                if lefts.pop().unwrap_or(0) != 0 {
                    return Err("\\left without \\right".into());
                }
            }
            Tok::Sub | Tok::Sup => {
                let sub = tok == Tok::Sub;
                if (sub && cur.sub) || (!sub && (cur.sup)) {
                    return Err(if sub { "double subscript".into() } else { "double superscript".into() });
                }
                if sub {
                    cur.sub = true;
                } else {
                    cur.sup = true;
                }
                match next_solid(i).map(|j| (j, &toks[j])) {
                    None | Some((_, Tok::Close | Tok::Amp | Tok::Sub | Tok::Sup)) => return Err("empty script".into()),
                    Some((j, Tok::Open)) => {
                        if next_solid(j + 1).is_none_or(|k| toks[k] == Tok::Close) {
                            return Err("empty script".into());
                        }
                        // the group is the script's argument: its own atom inside
                        i = j + 1;
                        groups.push(Scripts::default());
                        lefts.push(0);
                        // keep the outer atom's script flags (pushed below the new group)
                    }
                    Some((j, Tok::Cmd(c))) => {
                        i = j + 1;
                        let c = c.clone();
                        if !COMMANDS.contains(&c.as_str()) {
                            return Err(format!("unknown command \\{c}"));
                        }
                        if arity(&c) > 0 {
                            return Err(format!("\\{c} as a bare script"));
                        }
                    }
                    Some((j, _)) => i = j + 1,
                }
            }
            Tok::Prime => {
                if cur.sup && !cur.prime {
                    return Err("double superscript".into());
                }
                cur.prime = true;
            }
            Tok::Amp => {
                if envs.is_empty() {
                    return Err("& outside an alignment".into());
                }
                *cur = Scripts::default();
            }
            Tok::Ch(_) => *cur = Scripts::default(),
            Tok::Cmd(c) => {
                if !COMMANDS.contains(&c.as_str()) {
                    return Err(format!("unknown command \\{c}"));
                }
                match c.as_str() {
                    "limits" | "nolimits" => continue,
                    "begin" | "end" => {
                        let Some((name, after)) = next_solid(i).and_then(|j| group_text(&toks, j)) else { return Err(format!("\\{c} without a name")) };
                        if !ENVS.contains(&name.as_str()) {
                            return Err(format!("unknown environment {name}"));
                        }
                        if c == "begin" {
                            envs.push(name);
                        } else if envs.pop().as_deref() != Some(name.as_str()) {
                            return Err(format!("\\end{{{name}}} without its \\begin"));
                        }
                        i = after;
                        // the column spec of an array
                        if c == "begin" && matches!(envs.last().map(String::as_str), Some("array" | "alignedat" | "subarray"))
                            && let Some((_, a)) = next_solid(i).and_then(|j| group_text(&toks, j))
                        {
                            i = a;
                        }
                        *groups.last_mut().unwrap() = Scripts::default();
                        continue;
                    }
                    "left" | "right" | "middle" | "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl" | "Bigr" | "biggl" | "biggr" | "Biggl" | "Biggr" => {
                        let Some(j) = next_solid(i) else { return Err(format!("\\{c} without a delimiter")) };
                        let d = match &toks[j] {
                            Tok::Ch(ch) => ch.to_string(),
                            Tok::Cmd(x) => format!("\\{x}"),
                            _ => String::new(),
                        };
                        if !DELIMS.contains(&d.as_str()) {
                            return Err(format!("\\{c} without a delimiter"));
                        }
                        i = j + 1;
                        let depth = lefts.last_mut().unwrap();
                        if c == "left" {
                            *depth += 1;
                        } else if c == "right" {
                            if *depth == 0 {
                                return Err("\\right without \\left".into());
                            }
                            *depth -= 1;
                        }
                        *groups.last_mut().unwrap() = Scripts::default();
                        continue;
                    }
                    _ => {}
                }
                // arguments must be there (a group or one token)
                let mut k = i;
                if c == "sqrt"
                    && let Some(j) = next_solid(k)
                    && toks[j] == Tok::Ch('[')
                {
                    k = (j..toks.len()).find(|&m| toks[m] == Tok::Ch(']')).map(|m| m + 1).ok_or("\\sqrt[ without ]")?;
                }
                for _ in 0..arity(&c) {
                    match next_solid(k).map(|j| (j, &toks[j])) {
                        None | Some((_, Tok::Close | Tok::Amp | Tok::Sub | Tok::Sup)) => return Err(format!("\\{c} without its argument")),
                        Some((j, Tok::Open)) => {
                            let mut depth = 0;
                            let mut m = j;
                            loop {
                                match toks.get(m) {
                                    Some(Tok::Open) => depth += 1,
                                    Some(Tok::Close) => {
                                        depth -= 1;
                                        if depth == 0 {
                                            break;
                                        }
                                    }
                                    None => return Err("unbalanced {".into()),
                                    _ => {}
                                }
                                m += 1;
                            }
                            k = m + 1;
                        }
                        Some((j, _)) => k = j + 1,
                    }
                }
                *groups.last_mut().unwrap() = Scripts::default();
            }
        }
    }
    if groups.len() != 1 {
        return Err("unbalanced {".into());
    }
    if lefts[0] != 0 {
        return Err("\\left without \\right".into());
    }
    if let Some(e) = envs.last() {
        return Err(format!("\\begin{{{e}}} without its \\end"));
    }
    Ok(())
}

/// [`check`], and brackets that pair up: a reading that drops a `(` or a `]` (an operator's
/// limits, a broken line) typesets but says something else.
pub fn check_reading(latex: &str) -> Result<(), String> {
    check(latex)?;
    // `~` between symbols is a mis-read ∼ (in LaTeX a space); \mathcal has no small letters
    if latex.contains('~') {
        return Err("~ for a relation".into());
    }
    if latex.match_indices("\\mathcal{").any(|(i, _)| latex[i + 9..].starts_with(|c: char| c.is_ascii_lowercase())) {
        return Err("\\mathcal on a small letter".into());
    }
    let mut stack: Vec<char> = vec![];
    let cs: Vec<char> = latex.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '\\' {
            // \{ \} pair like brackets; other escapes and command names are skipped
            match cs.get(i + 1) {
                Some('{') => stack.push('{'),
                Some('}') if stack.pop() != Some('{') => return Err("unpaired \\}".into()),
                _ => {}
            }
            i += 2;
            while i < cs.len() && cs[i].is_ascii_alphabetic() {
                i += 1;
            }
            continue;
        }
        match c {
            '(' | '[' => stack.push(c),
            ')' if stack.pop() != Some('(') => return Err("unpaired )".into()),
            ']' if stack.pop() != Some('[') => return Err("unpaired ]".into()),
            _ => {}
        }
        i += 1;
    }
    if stack.is_empty() { Ok(()) } else { Err(format!("unpaired {}", stack[stack.len() - 1])) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_katex_typesets_passes() {
        for ok in [
            "\\mathcal{L}_{\\mathrm{D}} = \\frac{1}{2}\\mathbb{E}_{x\\sim P_{\\mathrm{real}}}[(D(x) - 1)^{2}]",
            "\\begin{aligned}a &= b + \\\\ &\\quad c\\end{aligned}",
            "x'_{p} = R_{p}(D_{p}(x_{p}))",
            "\\|x - \\tilde{x}\\|_{2}^{2}",
            "\\left( \\sum_{i=1}^{N} w_{i} \\right)",
            "f(x) = \\begin{cases} 1 & x > 0 \\\\ 0 & \\text{otherwise} \\end{cases}",
            "\\sqrt[3]{x} + \\operatorname{softmax}(z)",
            "\\theta'' + x^{\\prime}",
        ] {
            assert_eq!(check(ok), Ok(()), "{ok}");
        }
    }

    #[test]
    fn what_katex_refuses_is_refused() {
        for (bad, why) in [
            ("\\mathcal{x}\u{303}_{p}", "combining"),
            ("\\mathbb{E}_{x}_{y}", "double subscript"),
            ("x^{2}'", "double superscript"),
            ("a = {b", "unbalanced"),
            ("a = b}", "unbalanced"),
            ("\\left( x", "\\left"),
            ("\\frac{1}", "argument"),
            ("x_{}", "empty"),
            ("a & b", "outside"),
            ("\\foo{x}", "unknown command"),
            ("\\begin{weird}x\\end{weird}", "unknown environment"),
            ("\\begin{aligned}x", "without its \\end"),
            ("𝑥 + 1", "character"),
            ("", "empty"),
        ] {
            let e = check(bad).expect_err(bad);
            assert!(e.contains(why), "{bad}: {e}");
        }
    }

    #[test]
    fn readings_need_paired_brackets() {
        assert!(check_reading("W(\\mathbb{P}_{r}, \\mathbb{P}_{\\theta} \\mathbb{E}[f(x)]").is_err());
        assert!(check_reading("[(D(x) - 1)^{2}]").is_ok());
        assert!(check_reading("\\{x\\} \\cup [a, b]").is_ok());
        assert!(check_reading("[a, b)").is_err(), "half-open intervals are rare in a PDF reading; refused");
        assert!(check_reading("\\mathbb{E}_{\\mathcal{x}_{p}~P}[x]").is_err());
        assert!(check_reading("\\mathcal{L}_{\\mathcal{D}}").is_ok());
    }
}
