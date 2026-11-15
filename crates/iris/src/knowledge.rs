//! Iris's ready-written help, shared by every app. The topics are written in the
//! `knowledge` folder (built into the crate): one file for each app and `shared.toml` for the
//! topics about more than one, each topic tagged with the apps it is about; see the notes at
//! the top of `shared.toml`. An app asks for the answers to a question with `find`, for the
//! nearest topics when nothing answers it with `closest`, and for its tips with `tips`.

use std::sync::OnceLock;

use crate::App;
use crate::matching::{self, Topic as Matchable};

/// The files the topics are written in, in the order they are read. A topic's place in that
/// order is the order tips are given in and which of two equally good answers comes first.
const FILES: [(&str, &str); 4] = [
    ("studio.toml", include_str!("knowledge/studio.toml")),
    ("canvas.toml", include_str!("knowledge/canvas.toml")),
    ("shared.toml", include_str!("knowledge/shared.toml")),
    ("develop.toml", include_str!("knowledge/develop.toml")),
];

/// One topic as written in the `knowledge` folder.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Topic {
    pub title: String,
    /// The answer, with `{key:command-id}` where a shortcut goes.
    pub body: String,
    /// Extra words people might ask with, for the matching only.
    #[serde(default)]
    pub keywords: String,
    /// The apps it is about.
    pub apps: Vec<App>,
    /// The command id, in those apps, that does what it describes.
    #[serde(default)]
    pub command: Option<String>,
    /// Whether it is one of the tips given when Iris's face is clicked.
    #[serde(default)]
    pub tip: bool,
}

#[derive(serde::Deserialize)]
struct File {
    topic: Vec<Topic>,
}

/// Every topic, in the order written. The files are part of the crate and a test reads them,
/// so they can only be unreadable in a build whose tests failed.
pub fn topics() -> &'static [Topic] {
    static TOPICS: OnceLock<Vec<Topic>> = OnceLock::new();
    TOPICS.get_or_init(|| FILES.iter().flat_map(|(name, text)| parse(text).unwrap_or_else(|problem| panic!("{name} is checked by the crate's tests: {problem}"))).collect())
}

fn parse(text: &str) -> Result<Vec<Topic>, toml::de::Error> {
    Ok(toml::from_str::<File>(text)?.topic)
}

/// A topic as an answer given in one app.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub title: String,
    /// The text, with shortcuts filled in, and starting "In Omina Edit: " (say) when it is about
    /// another app.
    pub body: String,
    /// The command that does it, only when the topic is about the app asking.
    pub command: Option<String>,
}

impl Topic {
    /// Whether the topic is about `app`.
    pub fn is_for(&self, app: App) -> bool {
        self.apps.contains(&app)
    }

    /// The topic answered in `app`. `key_for` gives a command's current shortcut, if it has
    /// one in `app`.
    pub fn answer_in(&self, app: App, key_for: &dyn Fn(&str) -> Option<String>) -> Answer {
        if self.is_for(app) {
            return Answer { title: self.title.clone(), body: fill_keys(&self.body, key_for), command: self.command.clone() };
        }
        // The shortcuts are another app's, which this one can't look up, so they are left out,
        // and one the sentence can't do without is called "its shortcut there".
        let body = fill_keys(&without_key_notes(&self.body), &|_| Some("its shortcut there".to_owned()));
        let elsewhere: Vec<&str> = self.apps.iter().map(|other| other.name()).collect();
        let place = format!("In {}", elsewhere.join(" and "));
        // A topic may already start by naming its app ("In Omina Edit, choose …").
        let body = if body.starts_with(&place) { body } else { format!("{place}: {body}") };
        Answer { title: self.title.clone(), body, command: None }
    }
}

/// `text` without its shortcut notes: each ` ({key:command-id})` is taken out whole.
fn without_key_notes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(" ({key:") {
        let Some(length) = rest[start..].find(')') else { break };
        out.push_str(&rest[..start]);
        rest = &rest[start + length + 1..];
    }
    out.push_str(rest);
    out
}

/// `text` with each `{key:command-id}` replaced by that command's shortcut, or by "its command
/// (it has no key yet)" when it has none.
fn fill_keys(text: &str, key_for: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{key:") {
        let Some(length) = rest[start..].find('}') else { break };
        out.push_str(&rest[..start]);
        let id = &rest[start + "{key:".len()..start + length];
        out.push_str(&key_for(id).unwrap_or_else(|| "its command (it has no key yet)".to_owned()));
        rest = &rest[start + length + 1..];
    }
    out.push_str(rest);
    out
}

/// The topics that answer `question`, best first, at most `limit`, as positions in `topics()`.
/// Topics about every app are searched, so Iris can say where something is done. Topics that
/// score the same keep the order they are written in.
pub fn find(question: &str, limit: usize) -> Vec<usize> {
    matching::answers(question, &matchable(), limit)
}

/// The topics nearest to a question that none of them answers, for "did you mean…": best
/// first, at most `limit`, as positions in `topics()`.
pub fn closest(question: &str, limit: usize) -> Vec<usize> {
    matching::closest(question, &matchable(), limit)
}

/// Every topic as the matching sees it.
fn matchable() -> Vec<Matchable<'static>> {
    topics().iter().map(|topic| Matchable { title: &topic.title, body: &topic.body, keywords: &topic.keywords }).collect()
}

/// The words of every topic's title and keywords, for putting a mistyped question right (see
/// `spelling`).
pub fn vocabulary() -> Vec<String> {
    topics().iter().flat_map(|topic| crate::spelling::words(&topic.title).chain(crate::spelling::words(&topic.keywords)).collect::<Vec<_>>()).collect()
}

/// The tips for `app`, in the order written.
pub fn tips(app: App) -> impl Iterator<Item = &'static Topic> {
    topics().iter().filter(move |topic| topic.tip && topic.is_for(app))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_knowledge_files_read_and_every_app_has_its_topics_written_sensibly() {
        let mut topics = Vec::new();
        for (name, text) in FILES {
            let read = parse(text).unwrap_or_else(|problem| panic!("{name} must read: {problem}"));
            // A file about one app holds only topics that are at least partly about it.
            if let Some(app) = App::ALL.into_iter().find(|app| name.starts_with(&format!("{app:?}").to_lowercase())) {
                assert!(read.iter().all(|topic| topic.is_for(app)), "{name} has a topic that isn't about {app:?}");
            }
            topics.extend(read);
        }
        assert!(!topics.is_empty());
        for topic in &topics {
            assert!(!topic.title.trim().is_empty() && !topic.body.trim().is_empty(), "{topic:?}");
            assert!(!topic.apps.is_empty(), "{} says which apps it is about", topic.title);
            // Every `{key:` is closed, or the reader would print it as it is. Other braces, such
            // as a recipe's "{text}", are plain text.
            for (at, _) in topic.body.match_indices("{key:") {
                let closed = topic.body[at..].find('}').is_some_and(|end| !topic.body[at + 1..at + end].contains('{'));
                assert!(closed, "{}: an unclosed {{key:", topic.title);
            }
        }
        for app in App::ALL {
            assert!(tips(app).count() > 3, "{app:?} has too few tips");
        }
        // A topic written twice would be answered twice.
        let mut titles: Vec<&str> = topics.iter().map(|topic| topic.title.as_str()).collect();
        titles.sort_unstable();
        let repeated: Vec<&&str> = titles.windows(2).filter(|pair| pair[0] == pair[1]).map(|pair| &pair[0]).collect();
        assert!(repeated.is_empty(), "written more than once: {repeated:?}");
    }

    #[test]
    fn shortcuts_are_filled_in() {
        let key_for = |id: &str| (id == "split-layer").then(|| "⌘⇧D".to_owned());
        assert_eq!(fill_keys("Press {key:split-layer}.", &key_for), "Press ⌘⇧D.");
        assert_eq!(fill_keys("Use {key:missing}.", &key_for), "Use its command (it has no key yet).");
        assert_eq!(fill_keys("No keys {here", &key_for), "No keys {here");
    }

    #[test]
    fn a_topic_about_another_app_says_where_and_offers_no_button() {
        let canvas = topics().iter().find(|topic| topic.title == "Animate an Omina Edit design").unwrap();
        let answer = canvas.answer_in(App::Studio, &|_| None);
        assert!(answer.body.starts_with("In Omina Edit"), "{}", answer.body);
        assert_eq!(answer.command, None);
        let split = topics().iter().find(|topic| topic.title == "Cut a clip in two").unwrap();
        assert_eq!(split.answer_in(App::Studio, &|_| None).command.as_deref(), Some("split-layer"));
        assert_eq!(split.answer_in(App::Canvas, &|_| None).command, None);
    }

    #[test]
    fn another_apps_answer_has_no_shortcut_it_cant_know_and_says_where_once() {
        let canvas = topics().iter().find(|topic| topic.title == "Animate an Omina Edit design").unwrap();
        // Studio's shortcuts don't include Canvas's commands.
        let body = canvas.answer_in(App::Studio, &|_| None).body;
        assert!(!body.contains("{key:") && !body.contains("no key yet") && !body.contains("()"), "{body}");
        assert_eq!(body.matches("In Omina Edit").count(), 1, "{body}");
        // A key the sentence is built around is named without pretending to know it.
        let zoom = Topic { title: "Zoom".to_owned(), body: "Press {key:toggle-zoom}, or double-click.".to_owned(), keywords: String::new(), apps: vec![App::Develop], command: None, tip: false };
        assert_eq!(zoom.answer_in(App::Canvas, &|_| None).body, "In Omina Develop: Press its shortcut there, or double-click.");
        // In Canvas itself the key is filled in.
        let body = canvas.answer_in(App::Canvas, &|id| (id == "bring-to-motion").then(|| "⌘⇧M".to_owned())).body;
        assert!(body.contains("Bring to Motion (⌘⇧M)"), "{body}");
    }

    #[test]
    fn questions_about_other_apps_find_them() {
        let titles = |question: &str| find(question, 5).into_iter().map(|index| topics()[index].title.as_str()).collect::<Vec<_>>();
        assert_eq!(titles("how do I animate my canvas design").first().copied(), Some("Animate an Omina Edit design"));
        assert!(titles("share colors between apps").contains(&"Use the same colors and text styles in every app"));
    }
}
