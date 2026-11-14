//! Unicode a model writes (≤, −, ⁺, α, ×, …) as LaTeX that pdflatex accepts.
//!
//! Generated text (drafted paragraphs, rewrites, filled results) goes through
//! [`to_latex`] before it is shown or written, so a draft PDF never stops on
//! `Unicode character ≤ (U+2264) not set up for use with LaTeX`.
//!
//! Outside math a symbol becomes `\ensuremath{…}` (safe in text and in math);
//! inside `$…$`, `\(…\)` and `\[…\]` the bare command is used. Super- and
//! subscript digits become `\textsuperscript{…}` / `\textsubscript{…}` in text
//! and `^{…}` / `_{…}` in math. Typographic punctuation that pdflatex's utf8
//! input already handles (– — “ ” ‘ ’ …) is left alone; CJK text is untouched.

/// A math command for `c`, without `\ensuremath`.
fn math_symbol(c: char) -> Option<&'static str> {
    Some(match c {
        '≤' | '⩽' => "\\leq",
        '≥' | '⩾' => "\\geq",
        '≠' => "\\neq",
        '≈' => "\\approx",
        '∼' | '〜' => "\\sim",
        '≃' => "\\simeq",
        '≡' => "\\equiv",
        '±' => "\\pm",
        '∓' => "\\mp",
        '×' => "\\times",
        '÷' => "\\div",
        '·' | '⋅' => "\\cdot",
        '−' => "-",
        '∞' => "\\infty",
        '∑' => "\\sum",
        '∏' => "\\prod",
        '∫' => "\\int",
        '√' => "\\surd",
        '∂' => "\\partial",
        '∇' => "\\nabla",
        '∈' => "\\in",
        '∉' => "\\notin",
        '⊂' => "\\subset",
        '⊆' => "\\subseteq",
        '∪' => "\\cup",
        '∩' => "\\cap",
        '∀' => "\\forall",
        '∃' => "\\exists",
        '→' => "\\rightarrow",
        '←' => "\\leftarrow",
        '↔' => "\\leftrightarrow",
        '⇒' => "\\Rightarrow",
        '⇔' => "\\Leftrightarrow",
        '↑' => "\\uparrow",
        '↓' => "\\downarrow",
        '°' => "^{\\circ}",
        '′' => "'",
        '∘' => "\\circ",
        'α' => "\\alpha",
        'β' => "\\beta",
        'γ' => "\\gamma",
        'δ' => "\\delta",
        'ε' => "\\epsilon",
        'ζ' => "\\zeta",
        'η' => "\\eta",
        'θ' => "\\theta",
        'ι' => "\\iota",
        'κ' => "\\kappa",
        'λ' => "\\lambda",
        'μ' | 'µ' => "\\mu",
        'ν' => "\\nu",
        'ξ' => "\\xi",
        'π' => "\\pi",
        'ρ' => "\\rho",
        'σ' => "\\sigma",
        'τ' => "\\tau",
        'υ' => "\\upsilon",
        'φ' => "\\phi",
        'χ' => "\\chi",
        'ψ' => "\\psi",
        'ω' => "\\omega",
        'Γ' => "\\Gamma",
        'Δ' => "\\Delta",
        'Θ' => "\\Theta",
        'Λ' => "\\Lambda",
        'Ξ' => "\\Xi",
        'Π' => "\\Pi",
        'Σ' => "\\Sigma",
        'Φ' => "\\Phi",
        'Ψ' => "\\Psi",
        'Ω' => "\\Omega",
        _ => return None,
    })
}

/// A superscript / subscript character as (is_super, plain text).
fn script(c: char) -> Option<(bool, &'static str)> {
    Some(match c {
        '⁰' => (true, "0"),
        '¹' => (true, "1"),
        '²' => (true, "2"),
        '³' => (true, "3"),
        '⁴' => (true, "4"),
        '⁵' => (true, "5"),
        '⁶' => (true, "6"),
        '⁷' => (true, "7"),
        '⁸' => (true, "8"),
        '⁹' => (true, "9"),
        '⁺' => (true, "+"),
        '⁻' => (true, "-"),
        'ⁿ' => (true, "n"),
        'ᵀ' => (true, "T"),
        '₀' => (false, "0"),
        '₁' => (false, "1"),
        '₂' => (false, "2"),
        '₃' => (false, "3"),
        '₄' => (false, "4"),
        '₅' => (false, "5"),
        '₆' => (false, "6"),
        '₇' => (false, "7"),
        '₈' => (false, "8"),
        '₉' => (false, "9"),
        '₊' => (false, "+"),
        '₋' => (false, "-"),
        _ => return None,
    })
}

/// Other characters pdflatex stumbles on, as text-mode LaTeX.
fn text_symbol(c: char) -> Option<&'static str> {
    Some(match c {
        '\u{00A0}' => "~",
        '\u{2009}' | '\u{202F}' => "\\,",
        '\u{200B}' | '\u{FEFF}' => "",
        '‰' => "\\textperthousand{}",
        '™' => "\\texttrademark{}",
        '®' => "\\textregistered{}",
        '©' => "\\textcopyright{}",
        '§' => "\\S{}",
        '†' => "\\textdagger{}",
        '‡' => "\\textdaggerdbl{}",
        '•' => "\\textbullet{}",
        _ => return None,
    })
}

/// `text` with the Unicode above written as LaTeX. Idempotent; ASCII and CJK pass through.
pub fn to_latex(text: &str) -> String {
    if text.is_ascii() {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 16);
    // inside $…$ / \(…\) / \[…\]
    let mut math: Option<&str> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // escapes: \$ and friends are text, \( \[ open math, \) \] close it
        if c == '\\' {
            if let Some(&n) = chars.get(i + 1) {
                match (n, math) {
                    ('(', None) => math = Some("\\)"),
                    ('[', None) => math = Some("\\]"),
                    (')', Some("\\)")) | (']', Some("\\]")) => math = None,
                    _ => {}
                }
                out.push(c);
                out.push(n);
                i += 2;
                continue;
            }
        }
        if c == '$' {
            let double = chars.get(i + 1) == Some(&'$');
            match math {
                None => math = Some(if double { "$$" } else { "$" }),
                Some("$$") if double => math = None,
                Some("$") => math = None,
                _ => {}
            }
            out.push('$');
            if double {
                out.push('$');
                i += 1;
            }
            i += 1;
            continue;
        }
        let in_math = math.is_some();
        if let Some((sup, s)) = script(c) {
            // a run of the same kind: x²³ → x^{23}
            let mut run = s.to_string();
            while let Some((sup2, s2)) = chars.get(i + 1).and_then(|&n| script(n)) {
                if sup2 != sup {
                    break;
                }
                run.push_str(s2);
                i += 1;
            }
            if in_math {
                out.push_str(&format!("{}{{{run}}}", if sup { '^' } else { '_' }));
            } else {
                out.push_str(&format!("\\text{}script{{{run}}}", if sup { "super" } else { "sub" }));
            }
        } else if let Some(m) = math_symbol(c) {
            if in_math {
                out.push_str(m);
                // a command needs a break before a following letter: \leq x, not \leqx
                if m.starts_with('\\') && m[1..].chars().all(|x| x.is_ascii_alphabetic()) && chars.get(i + 1).is_some_and(|n| n.is_ascii_alphabetic()) {
                    out.push(' ');
                }
            } else {
                out.push_str(&format!("\\ensuremath{{{m}}}"));
            }
        } else if let Some(t) = text_symbol(c).filter(|_| !in_math) {
            out.push_str(t);
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

/// Generated prose as LaTeX: [`to_latex`], and a bare `%` (a model's "39.1%") escaped, since in
/// LaTeX it comments out the rest of the line (text lost, braces left open). Only for text a model
/// wrote: an author's own file may hold real comments.
pub fn prose_to_latex(text: &str) -> String {
    let t = to_latex(text);
    let mut out = String::with_capacity(t.len() + 8);
    let mut prev = '\0';
    for c in t.chars() {
        if c == '%' && prev != '\\' {
            out.push('\\');
        }
        out.push(c);
        // `\\%` is a line break then a comment; rare in prose, kept as is
        prev = if prev == '\\' && c == '\\' { '\0' } else { c };
    }
    out
}

/// Characters of `text` that [`to_latex`] would change (for a check message).
pub fn unsupported(text: &str) -> Vec<char> {
    let mut v: Vec<char> = text.chars().filter(|&c| math_symbol(c).is_some() || script(c).is_some() || text_symbol(c).is_some()).collect();
    v.sort();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_symbols_become_ensuremath_and_scripts() {
        assert_eq!(to_latex("AUC ≥ 0.90 and p ≤ 0.05"), "AUC \\ensuremath{\\geq} 0.90 and p \\ensuremath{\\leq} 0.05");
        assert_eq!(to_latex("a drop of −0.039"), "a drop of \\ensuremath{-}0.039");
        assert_eq!(to_latex("CD4⁺ cells, H₂O, 10³ images"), "CD4\\textsuperscript{+} cells, H\\textsubscript{2}O, 10\\textsuperscript{3} images");
        assert_eq!(to_latex("λ = 0.5, 25 °C, 3 × 3"), "\\ensuremath{\\lambda} = 0.5, 25 \\ensuremath{^{\\circ}}C, 3 \\ensuremath{\\times} 3");
    }

    #[test]
    fn math_keeps_bare_commands() {
        assert_eq!(to_latex("$x ≤ y$ and $λx$"), "$x \\leq y$ and $\\lambda x$");
        assert_eq!(to_latex("\\(a²\\) then b²"), "\\(a^{2}\\) then b\\textsuperscript{2}");
        assert_eq!(to_latex("cost \\$5 ≤ $x$"), "cost \\$5 \\ensuremath{\\leq} $x$");
    }

    #[test]
    fn a_models_percent_is_escaped_once() {
        assert_eq!(prose_to_latex("39.1% on MIMIC and 13.7\\% on VA, p ≤ 0.05"), "39.1\\% on MIMIC and 13.7\\% on VA, p \\ensuremath{\\leq} 0.05");
        let once = prose_to_latex("52.2%");
        assert_eq!(prose_to_latex(&once), once);
    }

    #[test]
    fn leaves_ascii_cjk_and_punctuation_and_is_idempotent() {
        for s in ["plain ascii $x_1$", "中文的句子，不改。", "“quoted” – ranges — and …"] {
            assert_eq!(to_latex(s), s);
        }
        let once = to_latex("p ≤ 0.05, x² and −1");
        assert_eq!(to_latex(&once), once);
        assert_eq!(unsupported("a ≤ b − c"), vec!['−', '≤']);
    }
}
