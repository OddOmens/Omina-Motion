//! What Iris can answer in Studio when asked for help: a fixed list of ready-written answers,
//! found by the words in the question. The list is Iris's shared knowledge (the `iris` crate's
//! `knowledge` folder, which Canvas and Develop read too), then every Studio command with its
//! current shortcut, then every tool and feature with what it does. Nothing is generated and
//! nothing is AI.

use iris::App;
use iris::matching::Topic;

use crate::app_settings::AppSettings;
use crate::features::{Feature, FeatureInfo, GROUPS};
use crate::keymap::{COMMANDS, Command, key_for};

/// The most answers shown for one question.
const MOST_ANSWERS: usize = 5;

/// One answer the assistant can give.
pub struct Answer {
    pub title: String,
    pub body: String,
    /// Extra words people might ask with, for the matching only.
    keywords: &'static str,
    /// The command that does what the answer describes, offered as a "Do It" button.
    pub command: Option<&'static Command>,
    /// The command's current shortcut as the Mac writes it, shown beside the answer.
    pub key: Option<String>,
}

impl Answer {
    /// The answer's title, followed by its command's key when it has one: "Easy ease (F9)".
    pub fn heading(&self) -> String {
        match &self.key {
            Some(key) => format!("{} ({key})", self.title),
            None => self.title.clone(),
        }
    }
}

/// What Settings says about a feature, if it is one that can be turned off.
fn feature_info(feature: Feature) -> Option<&'static FeatureInfo> {
    GROUPS.iter().flat_map(|(_, features)| features.iter()).find(|info| info.feature == feature)
}

/// A topic of Iris's shared knowledge as Studio answers it: with shortcuts as they are set
/// now, and a button only when the topic is about Studio and its command exists here.
fn from_knowledge(topic: &'static iris::knowledge::Topic, settings: &AppSettings) -> Answer {
    let answer = topic.answer_in(App::Studio, &|id| key_for(&settings.shortcuts, id));
    let command = answer.command.as_deref().and_then(crate::keymap::command);
    let key = command.and_then(|command| key_for(&settings.shortcuts, command.id));
    Answer { title: answer.title, body: answer.body, keywords: &topic.keywords, command, key }
}

/// Every answer the assistant has, the shared knowledge first. Commands and features that are
/// turned off in Settings are left out, as they are everywhere else, and so are topics about
/// the other apps when Settings ▸ Iris says to stick to Studio.
fn all(settings: &AppSettings) -> Vec<Answer> {
    let mut answers: Vec<Answer> = iris::knowledge::topics()
        .iter()
        .filter(|topic| settings.assistant_other_apps || topic.is_for(App::Studio))
        .map(|topic| from_knowledge(topic, settings))
        .collect();
    let mut told: Vec<Feature> = Vec::new();
    for command in COMMANDS {
        let feature = Feature::of_command(command.id);
        if feature.is_some_and(|feature| !settings.feature_on(feature)) {
            continue;
        }
        let key = key_for(&settings.shortcuts, command.id);
        // A command that belongs to a tool or feature says what that does.
        let about = feature.and_then(feature_info).map(|info| info.about);
        told.extend(feature);
        let shortcut = match &key {
            Some(key) => format!("Its key is {key}."),
            None => "It has no key yet; Settings ▸ Keyboard Shortcuts can give it one.".to_owned(),
        };
        let body = match about {
            Some(about) => format!("{about} {shortcut}"),
            None => format!("One of the {} commands. {shortcut}", command.group),
        };
        answers.push(Answer { title: command.name.to_owned(), body, keywords: "", command: command.works_in_editor().then_some(command), key });
    }
    for info in GROUPS.iter().flat_map(|(_, features)| features.iter()) {
        if settings.feature_on(info.feature) && !told.contains(&info.feature) {
            answers.push(Answer { title: info.name.to_owned(), body: info.about.to_owned(), keywords: "", command: None, key: None });
        }
    }
    answers
}

/// The answers to a question, best first. Empty when nothing fits.
pub fn find(question: &str, settings: &AppSettings) -> Vec<Answer> {
    let mut answers: Vec<Option<Answer>> = all(settings).into_iter().map(Some).collect();
    let topics: Vec<Topic> = answers.iter().flatten().map(|answer| Topic { title: &answer.title, body: &answer.body, keywords: answer.keywords }).collect();
    let found = iris::matching::answers(question, &topics, MOST_ANSWERS);
    drop(topics);
    found.into_iter().filter_map(|index| answers[index].take()).collect()
}

/// The most answers offered as "did you mean" when nothing answers a question.
const MOST_NEAREST: usize = 3;

/// The answers nearest to a question that none of them answers, best first, for offering
/// when the assistant doesn't know. Empty when nothing shares a word with it.
pub fn closest(question: &str, settings: &AppSettings) -> Vec<Answer> {
    let mut answers: Vec<Option<Answer>> = all(settings).into_iter().map(Some).collect();
    let topics: Vec<Topic> = answers.iter().flatten().map(|answer| Topic { title: &answer.title, body: &answer.body, keywords: answer.keywords }).collect();
    let found = iris::matching::closest(question, &topics, MOST_NEAREST);
    drop(topics);
    found.into_iter().filter_map(|index| answers[index].take()).collect()
}

/// The tip the assistant gives on the `turn`th click of its face: Studio's tips from the shared
/// knowledge in order, starting over after the last.
pub fn tip(turn: usize, settings: &AppSettings) -> Answer {
    let tips: Vec<&'static iris::knowledge::Topic> = iris::knowledge::tips(App::Studio).collect();
    from_knowledge(tips[turn % tips.len()], settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_find_how_tos_commands_and_features() {
        let settings = AppSettings::default();
        let titles = |question: &str| find(question, &settings).into_iter().map(|answer| answer.title).collect::<Vec<_>>();
        assert_eq!(titles("how do I make my motion smoother?").first().map(String::as_str), Some("Make motion smoother"));
        assert!(titles("how do I split a clip").iter().any(|title| title == "Split at the playhead"));
        assert!(titles("stabilize shaky footage").iter().any(|title| title == "Stabilize"), "features answer too");
        assert!(titles("what is the meaning of life").is_empty());
    }

    #[test]
    fn answers_carry_the_current_key_and_skip_what_is_turned_off() {
        let mut settings = AppSettings::default();
        let pen = |settings: &AppSettings| find("pen", settings).into_iter().find(|answer| answer.title == "Pen");
        let answer = pen(&settings).expect("the Pen tool is on to begin with");
        assert_eq!(answer.key, key_for(&settings.shortcuts, "pen-tool"));
        assert!(answer.command.is_some(), "it can be done from the answer");
        settings.set_feature(Feature::PenTool, false);
        assert!(pen(&settings).is_none());
        // Quitting isn't offered as a button.
        let quit = find("quit studio", &settings).into_iter().find(|answer| answer.title == "Quit Omina Motion").unwrap();
        assert!(quit.command.is_none());
    }

    #[test]
    fn questions_about_canvas_are_answered_without_a_button() {
        let settings = AppSettings::default();
        let answer = find("how do I animate my canvas design", &settings).into_iter().next().unwrap();
        assert!(answer.body.starts_with("In Omina Edit"), "{}", answer.body);
        assert!(answer.command.is_none());
    }

    #[test]
    fn other_apps_can_be_left_out() {
        let mut settings = AppSettings { assistant_other_apps: false, ..AppSettings::default() };
        assert!(find("how do I animate my canvas design", &settings).iter().all(|answer| !answer.body.starts_with("In Omina Edit")));
        settings.assistant_other_apps = true;
        assert!(find("how do I animate my canvas design", &settings).iter().any(|answer| answer.body.starts_with("In Omina Edit")));
    }

    #[test]
    fn tips_go_round_the_how_tos() {
        let settings = AppSettings::default();
        let count = iris::knowledge::tips(App::Studio).count();
        assert_eq!(tip(0, &settings).title, tip(count, &settings).title);
        assert_ne!(tip(0, &settings).title, tip(1, &settings).title);
    }
}
