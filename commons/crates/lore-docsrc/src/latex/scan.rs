//! Small LaTeX scanning helpers: comments, balanced groups, environments,
//! top-level splitting. Byte offsets everywhere (all delimiters are ASCII).

/// Remove `%` comments (not `\%`), keeping every newline so line numbers stay valid.
pub fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, line) in s.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let b = line.as_bytes();
        let mut cut = line.len();
        let mut j = 0;
        while j < b.len() {
            if b[j] == b'\\' {
                j += 2;
                continue;
            }
            if b[j] == b'%' {
                cut = j;
                break;
            }
            j += 1;
        }
        out.push_str(&line[..cut]);
    }
    out
}

pub fn skip_ws(s: &str, mut pos: usize) -> usize {
    let b = s.as_bytes();
    while pos < b.len() && (b[pos] as char).is_ascii_whitespace() {
        pos += 1;
    }
    pos
}

/// A `{…}` group starting at `pos` (after optional whitespace).
/// Returns (inner start, inner end, position after `}`).
pub fn read_group(s: &str, pos: usize) -> Option<(usize, usize, usize)> {
    let b = s.as_bytes();
    let p = skip_ws(s, pos);
    if p >= b.len() || b[p] != b'{' {
        return None;
    }
    let mut depth = 0i32;
    let mut i = p;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((p + 1, i, i + 1));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// A `[…]` optional argument at `pos` (after optional whitespace), braces respected.
pub fn read_optional(s: &str, pos: usize) -> Option<(usize, usize, usize)> {
    let b = s.as_bytes();
    let p = skip_ws(s, pos);
    if p >= b.len() || b[p] != b'[' {
        return None;
    }
    let mut depth = 0i32;
    let mut i = p + 1;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b']' if depth == 0 => return Some((p + 1, i, i + 1)),
            _ => {}
        }
        i += 1;
    }
    None
}

/// The text of the `{…}` group at `pos`, and the position after it.
pub fn group_text(s: &str, pos: usize) -> Option<(&str, usize)> {
    read_group(s, pos).map(|(a, b, after)| (&s[a..b], after))
}

/// Skip any number of `[…]` optional arguments.
pub fn skip_optionals(s: &str, mut pos: usize) -> usize {
    while let Some((_, _, after)) = read_optional(s, pos) {
        pos = after;
    }
    pos
}

/// Find the next `\name` (not followed by a letter, `*` allowed only when
/// `star_ok`) at or after `from`. Returns (start of backslash, position after name[*]).
pub fn find_command(s: &str, name: &str, from: usize) -> Option<(usize, usize, bool)> {
    let pat = format!("\\{name}");
    let b = s.as_bytes();
    let mut i = from;
    while let Some(off) = s.get(i..)?.find(&pat) {
        let start = i + off;
        // not escaped (`\\name` is a line break followed by text)
        let escaped = start > 0 && b[start - 1] == b'\\' && !(start > 1 && b[start - 2] == b'\\');
        let mut end = start + pat.len();
        let next = b.get(end).copied();
        if escaped || next.is_some_and(|c| c.is_ascii_alphabetic()) {
            i = start + 1;
            continue;
        }
        let star = next == Some(b'*');
        if star {
            end += 1;
        }
        return Some((start, end, star));
    }
    None
}

/// Given the position right after `\begin{name}`, find the matching
/// `\end{name}`. Returns (start of `\end`, position after `\end{name}`).
pub fn find_env_end(s: &str, name: &str, from: usize) -> Option<(usize, usize)> {
    let open = format!("\\begin{{{name}}}");
    let close = format!("\\end{{{name}}}");
    let mut depth = 1;
    let mut i = from;
    loop {
        let o = s[i..].find(&open).map(|x| x + i);
        let c = s[i..].find(&close).map(|x| x + i)?;
        match o {
            Some(o) if o < c => {
                depth += 1;
                i = o + open.len();
            }
            _ => {
                depth -= 1;
                if depth == 0 {
                    return Some((c, c + close.len()));
                }
                i = c + close.len();
            }
        }
    }
}

/// One `\begin{env}…\end{env}` found in a text.
#[derive(Debug, Clone)]
pub struct Env {
    pub name: String,
    /// Offset of `\begin`.
    pub start: usize,
    /// Offset right after `\begin{name}`.
    pub body_start: usize,
    /// Offset of `\end{name}`.
    pub body_end: usize,
    /// Offset after `\end{name}`.
    pub end: usize,
}

impl Env {
    pub fn body<'a>(&self, s: &'a str) -> &'a str {
        &s[self.body_start..self.body_end]
    }
    pub fn whole<'a>(&self, s: &'a str) -> &'a str {
        &s[self.start..self.end]
    }
}

/// The next `\begin{…}` at or after `from` whose name satisfies `want`.
pub fn next_env(s: &str, from: usize, want: &dyn Fn(&str) -> bool) -> Option<Env> {
    let mut i = from;
    while let Some((start, after, _)) = find_command(s, "begin", i) {
        let Some((name, body_start)) = group_text(s, after) else {
            i = after;
            continue;
        };
        let name = name.trim().to_string();
        if want(&name)
            && let Some((body_end, end)) = find_env_end(s, &name, body_start)
        {
            return Some(Env {
                name,
                start,
                body_start,
                body_end,
                end,
            });
        }
        i = body_start;
    }
    None
}

/// All top-level environments of the given names (nested ones are skipped).
pub fn envs(s: &str, want: &dyn Fn(&str) -> bool) -> Vec<Env> {
    let mut out = vec![];
    let mut i = 0;
    while let Some(e) = next_env(s, i, want) {
        i = e.end;
        out.push(e);
    }
    out
}

/// Split at `sep` (e.g. `\\` or `&`) outside braces and nested environments.
pub fn split_top<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut depth = 0i32;
    let mut env_depth = 0i32;
    let mut last = 0;
    let mut i = 0;
    while i < b.len() {
        if s[i..].starts_with("\\begin{") {
            env_depth += 1;
            i += 6;
            continue;
        }
        if s[i..].starts_with("\\end{") {
            env_depth -= 1;
            i += 4;
            continue;
        }
        if depth == 0 && env_depth == 0 && s[i..].starts_with(sep) {
            // `\&` is a literal ampersand
            let escaped = sep == "&" && i > 0 && b[i - 1] == b'\\';
            if !escaped {
                out.push(&s[last..i]);
                i += sep.len();
                last = i;
                continue;
            }
        }
        match b[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    out.push(&s[last..]);
    out
}

/// All arguments of every `\name{…}` occurrence (optional args skipped).
pub fn command_args<'a>(s: &'a str, name: &str) -> Vec<(usize, &'a str)> {
    let mut out = vec![];
    let mut i = 0;
    while let Some((start, after, _)) = find_command(s, name, i) {
        let p = skip_optionals(s, after);
        match group_text(s, p) {
            Some((t, next)) => {
                out.push((start, t));
                i = next;
            }
            None => i = after,
        }
    }
    out
}

/// The first `\name{…}` argument.
pub fn first_arg<'a>(s: &'a str, name: &str) -> Option<&'a str> {
    command_args(s, name).into_iter().next().map(|(_, t)| t)
}

/// Remove every `\name[…]{…}` occurrence (with `nargs` brace groups).
pub fn remove_command(s: &str, name: &str, nargs: usize) -> String {
    let mut out = String::new();
    let mut i = 0;
    while let Some((start, after, _)) = find_command(s, name, i) {
        out.push_str(&s[i..start]);
        let mut p = skip_optionals(s, after);
        for _ in 0..nargs {
            match read_group(s, p) {
                Some((_, _, next)) => p = next,
                None => break,
            }
        }
        i = p;
    }
    out.push_str(&s[i..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_keep_lines() {
        assert_eq!(strip_comments("a % x\n50\\% b % c\nd"), "a \n50\\% b \nd");
    }

    #[test]
    fn groups_and_optionals() {
        let s = r"\caption[short]{A {nested} \} brace} rest";
        let (_, after, _) = find_command(s, "caption", 0).unwrap();
        let p = skip_optionals(s, after);
        let (t, _) = group_text(s, p).unwrap();
        assert_eq!(t, r"A {nested} \} brace");
    }

    #[test]
    fn commands_are_whole_words() {
        assert!(find_command(r"\captionsetup{x}", "caption", 0).is_none());
        let (_, _, star) = find_command(r"\section*{X}", "section", 0).unwrap();
        assert!(star);
    }

    #[test]
    fn nested_envs() {
        let s = r"\begin{figure}a\begin{figure}b\end{figure}c\end{figure}z";
        let e = next_env(s, 0, &|n| n == "figure").unwrap();
        assert_eq!(e.body(s), r"a\begin{figure}b\end{figure}c");
        assert_eq!(&s[e.end..], "z");
    }

    #[test]
    fn split_rows_and_cells() {
        let rows = split_top(
            r"a & \textbf{b \& c} \\ \begin{tabular}{c} x \\ y \end{tabular} & d \\",
            "\\\\",
        );
        assert_eq!(rows.len(), 3);
        let cells = split_top(rows[0], "&");
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[1].trim(), r"\textbf{b \& c}");
        assert_eq!(split_top(rows[1], "&").len(), 2);
    }
}
