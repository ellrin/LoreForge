//! The formats: fixed shapes of knowledge. Each implements [`crate::Format`]; a format that
//! reads an app's existing data also gives [`crate::Adapter`]s.
//!
//! * `table` (K2): pattern, expression, wording, rule, exemplar.
//! * `text` (K3): note, bundle, layout.
//! * `figure` (K4): figure (component / module / figure; entity or reference).

pub mod figure;
pub mod table;
pub mod text;

use std::collections::BTreeMap;

use crate::types::Format;

/// Format id → format: the one table `intake` and `select` read.
pub type Formats = BTreeMap<String, Box<dyn Format>>;

/// Every format this crate implements, by id: the five row formats, the three text formats and
/// the figure format. `card` is not here yet (knowledge cards stay in L-Brain's own store).
pub fn all() -> Formats {
    let mut out = Formats::new();
    for f in table::all().into_iter().chain(text::all()).chain(std::iter::once(Box::new(figure::Figure) as Box<dyn Format>)) {
        out.insert(f.def().id, f);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::format;

    #[test]
    fn every_format_id_has_one_format() {
        let all = all();
        let ids = [format::PATTERN, format::EXPRESSION, format::WORDING, format::RULE, format::EXEMPLAR, format::VARIANT, format::NOTE, format::BUNDLE, format::LAYOUT, format::FIGURE];
        assert_eq!(all.len(), ids.len(), "one entry per format id");
        for id in ids {
            let f = all.get(id).unwrap_or_else(|| panic!("no format for {id}"));
            assert_eq!(f.def().id, id);
        }
        assert!(!all.contains_key(format::CARD), "card is excluded for now");
        assert_eq!(table::all().len() + text::all().len() + 1, all.len(), "no two formats share an id");
    }
}
