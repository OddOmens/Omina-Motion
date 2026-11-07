//! The list of what an export left out or simplified. Each kind of loss is one line, naming
//! the layers it affects, so a composition with forty effects gives a short, readable notice
//! rather than forty lines.

/// Losses collected while exporting, in the order they were first met.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Notes {
    topics: Vec<(String, Vec<String>)>,
}

/// How many names a line lists before summing up the rest.
const NAMES_SHOWN: usize = 4;

impl Notes {
    /// Records that `topic` (such as "Effects were left out") applies to `what` (usually a
    /// layer's name). The same pair twice is recorded once.
    pub fn add(&mut self, topic: &str, what: impl Into<String>) {
        let what = what.into();
        let names = match self.topics.iter_mut().find(|(known, _)| known == topic) {
            Some((_, names)) => names,
            None => {
                self.topics.push((topic.to_owned(), Vec::new()));
                &mut self.topics.last_mut().expect("just pushed").1
            }
        };
        if !what.is_empty() && !names.contains(&what) {
            names.push(what);
        }
    }

    /// Records a loss that isn't about particular layers.
    pub fn add_general(&mut self, topic: &str) {
        self.add(topic, "");
    }

    /// One line per topic: "Effects were left out: “Glow” on “Title”, … and 3 more."
    pub fn into_lines(self) -> Vec<String> {
        self.topics
            .into_iter()
            .map(|(topic, names)| {
                if names.is_empty() {
                    return format!("{topic}.");
                }
                let shown: Vec<String> = names.iter().take(NAMES_SHOWN).cloned().collect();
                let more = names.len().saturating_sub(NAMES_SHOWN);
                let list = if more > 0 { format!("{} and {more} more", shown.join(", ")) } else { shown.join(", ") };
                format!("{topic}: {list}.")
            })
            .collect()
    }
}

/// A layer or effect name in curly quotes, for the notes.
pub(crate) fn quoted(name: &str) -> String {
    format!("“{name}”")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn losses_are_grouped_by_kind_and_long_lists_summed_up() {
        let mut notes = Notes::default();
        for name in ["A", "B", "C", "D", "E", "F", "A"] {
            notes.add("Video layers were skipped", quoted(name));
        }
        notes.add_general("Sound was left out");
        let lines = notes.into_lines();
        assert_eq!(lines, ["Video layers were skipped: “A”, “B”, “C”, “D” and 2 more.", "Sound was left out."]);
    }
}
