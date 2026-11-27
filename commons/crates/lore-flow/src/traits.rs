//! What kind of figure a figure is, said at the start: its scale (a whole architecture, several
//! modules side by side, one part that stands alone), what it stresses (the process: who does
//! what, in what order, how it is decided; or the algorithm: networks, tensors, losses) and the
//! shapes it is laid in (parallel rows, panels, an overview strip, a decision tree, one chain).
//! A worked example is picked by these, not by its subject: what carries over from an example
//! is its structure.

use serde::{Deserialize, Serialize};

pub const SCALES: [&str; 3] = ["whole", "modules", "part"];
pub const EMPHASES: [&str; 2] = ["process", "algorithm"];
pub const SHAPES: [&str; 5] = ["rows", "panels", "strip", "tree", "chain"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Traits {
    /// One of [`SCALES`]; empty = not said.
    pub scale: String,
    /// One of [`EMPHASES`]; empty = not said.
    pub emphasis: String,
    /// Any of [`SHAPES`].
    pub shape: Vec<String>,
    /// The parts of the figure that could stand alone as a figure (a panel's title).
    pub parts: Vec<String>,
}

impl Traits {
    /// How alike two figures are in kind: what they stress counts most (a process figure is
    /// no model for an algorithm), then their scale, then each shape they share. A figure
    /// with parts that stand alone can serve a `part` figure. 0 = nothing in common.
    pub fn like(&self, want: &Traits) -> u32 {
        let said = |a: &str, b: &str| !a.is_empty() && a == b;
        let mut n = 0;
        if said(&self.emphasis, &want.emphasis) {
            n += 4;
        }
        if said(&self.scale, &want.scale) {
            n += 3;
        } else if want.scale == "part" && !self.parts.is_empty() {
            n += 1;
        }
        n + want.shape.iter().filter(|s| self.shape.contains(s)).count() as u32
    }
}

/// The examples to show for a figure of this kind, most alike first, at most `limit`; those
/// with nothing in common are left out. When nothing is said of the figure wanted, the first
/// `limit` as they come. Stable: equals keep their order.
pub fn pick<T>(examples: Vec<(Traits, T)>, want: &Traits, limit: usize) -> Vec<T> {
    if *want == Traits::default() {
        return examples.into_iter().take(limit).map(|x| x.1).collect();
    }
    let mut v: Vec<(u32, T)> = examples.into_iter().map(|(t, x)| (t.like(want), x)).filter(|x| x.0 > 0).collect();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.into_iter().take(limit).map(|x| x.1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(scale: &str, emphasis: &str, shape: &[&str], parts: &[&str]) -> Traits {
        Traits { scale: scale.into(), emphasis: emphasis.into(), shape: shape.iter().map(|s| s.to_string()).collect(), parts: parts.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn an_example_is_picked_by_what_kind_of_figure_it_is() {
        let all = || vec![(t("whole", "algorithm", &["rows"], &[]), "rows of a network"), (t("whole", "process", &["strip", "panels", "tree"], &["a", "b"]), "a pipeline"), (t("modules", "algorithm", &["panels"], &["x"]), "modules")];
        // a process figure: the pipeline first, and the algorithm figures only for a shape they share
        assert_eq!(pick(all(), &t("whole", "process", &["tree"], &[]), 2), vec!["a pipeline", "rows of a network"]);
        // a part that stands alone, about an algorithm: the examples that have such parts come before one that has none
        assert_eq!(pick(all(), &t("part", "algorithm", &[], &[]), 3), vec!["modules", "rows of a network", "a pipeline"]);
        // nothing said: as they come
        assert_eq!(pick(all(), &Traits::default(), 2), vec!["rows of a network", "a pipeline"]);
        // nothing in common: nothing shown
        assert!(pick(vec![(t("whole", "algorithm", &["rows"], &[]), 1)], &t("part", "process", &["tree"], &[]), 4).is_empty());
    }
}
