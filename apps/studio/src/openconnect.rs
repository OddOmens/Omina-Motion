//! Reading device frame looks from OpenConnect, the App Store screenshot studio. Each app there
//! has a frame style (body color, outline color and width, shadow) and a background color; the
//! Mockups panel offers them as finishes, so mockups match the app's store screenshots.
//!
//! The request goes through the system's `curl`, as model downloads do. The token comes from
//! OpenConnect's own file (`~/.config/openconnect/token`) and is handed to `curl` on its standard
//! input, so it never appears on a command line or on screen.

use std::io::Write as _;
use std::process::{Command, Stdio};

use studio_core::mockups::Finish;
use studio_core::model::Rgba;

/// One OpenConnect app's device look.
#[derive(Clone, Debug, PartialEq)]
pub struct AppFinish {
    pub name: String,
    pub finish: Finish,
}

/// Where OpenConnect keeps its API token.
fn token_path() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(|home| std::path::PathBuf::from(home).join(".config/openconnect/token"))
}

/// Every app's finish from the OpenConnect API at `api` (its address, ending in `/api`).
pub fn fetch(api: &str) -> Result<Vec<AppFinish>, String> {
    let api = api.trim().trim_end_matches('/');
    if !(api.starts_with("http://") || api.starts_with("https://")) {
        return Err("The OpenConnect address should start with http:// or https://.".into());
    }
    let token = token_path().and_then(|path| std::fs::read_to_string(path).ok()).map(|token| token.trim().to_owned()).filter(|token| !token.is_empty());
    let Some(token) = token else { return Err("No OpenConnect token was found in ~/.config/openconnect/token.".into()) };
    // A token with quotes or line breaks would break out of the config line.
    if token.chars().any(|c| c == '"' || c == '\\' || c.is_control()) {
        return Err("The OpenConnect token file holds characters a token can't have.".into());
    }
    let mut curl = Command::new("curl")
        .args(["-sSf", "--max-time", "15", "-K", "-"])
        .arg(format!("{api}/data"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("could not start curl: {error}"))?;
    if let Some(mut stdin) = curl.stdin.take() {
        writeln!(stdin, "header = \"Authorization: Bearer {token}\"").map_err(|error| format!("could not talk to curl: {error}"))?;
    }
    let output = curl.wait_with_output().map_err(|error| format!("curl failed: {error}"))?;
    if !output.status.success() {
        return Err("OpenConnect didn't answer. Check the address and that it is running.".into());
    }
    let text = String::from_utf8(output.stdout).map_err(|_| "OpenConnect sent something that isn't text.".to_owned())?;
    let finishes = finishes_from_json(&text)?;
    if finishes.is_empty() {
        return Err("OpenConnect has no apps with a device frame yet.".into());
    }
    Ok(finishes)
}

/// The finishes in an `/api/data` answer: each app's `style.frame` and its primary background.
/// Apps without a frame style are skipped.
pub fn finishes_from_json(text: &str) -> Result<Vec<AppFinish>, String> {
    let apps: serde_json::Value = serde_json::from_str(text).map_err(|_| "OpenConnect's answer wasn't readable.".to_owned())?;
    let color = |value: Option<&serde_json::Value>| value.and_then(serde_json::Value::as_str).and_then(Rgba::parse_hex);
    Ok(apps
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|app| {
            let name = app.get("name")?.as_str()?.trim();
            let style = app.get("style")?;
            let frame = style.get("frame")?;
            let finish = Finish {
                body: color(frame.get("deviceColor"))?,
                outline: color(frame.get("strokeColor")).unwrap_or(Rgba::hex(0x48484a)),
                outline_width: frame.get("strokeWidth").and_then(serde_json::Value::as_f64).unwrap_or(4.0).clamp(0.0, 40.0) as f32,
                shadow: frame.get("shadow").and_then(serde_json::Value::as_f64).unwrap_or(0.55).clamp(0.0, 1.0) as f32,
                background: color(style.get("palettes").and_then(|palettes| palettes.get("primary")).and_then(|primary| primary.get("background"))),
            };
            (!name.is_empty()).then(|| AppFinish { name: name.to_owned(), finish })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_apps_frame_becomes_a_finish() {
        let answer = r##"[
            {"name": "Plinthio", "style": {"frame": {"kind": "device", "deviceColor": "#1c2433", "strokeColor": "#a0b8ce", "strokeWidth": 4, "shadow": 0.6},
                                           "palettes": {"primary": {"background": "#16202f"}}}},
            {"name": "No Frame", "style": {}},
            {"name": "Bare", "style": {"frame": {"deviceColor": "#111113"}}}
        ]"##;
        let finishes = finishes_from_json(answer).unwrap();
        assert_eq!(finishes.iter().map(|app| app.name.as_str()).collect::<Vec<_>>(), ["Plinthio", "Bare"]);
        let plinthio = finishes[0].finish;
        assert_eq!((plinthio.body, plinthio.outline, plinthio.outline_width, plinthio.shadow), (Rgba::hex(0x1c2433), Rgba::hex(0xa0b8ce), 4.0, 0.6));
        assert_eq!(plinthio.background, Some(Rgba::hex(0x16202f)));
        assert_eq!(finishes[1].finish.background, None, "no background color: see-through");
        assert!(finishes_from_json("not json").is_err());
    }

    /// Reads a real OpenConnect: `OPENCONNECT_API=http://host:3100/studio/api cargo test -p studio
    /// reads_a_real_openconnect -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs an OpenConnect server; set OPENCONNECT_API"]
    fn reads_a_real_openconnect() {
        let finishes = fetch(&std::env::var("OPENCONNECT_API").unwrap()).unwrap();
        for app in &finishes {
            println!("{}: body {} outline {}", app.name, app.finish.body.to_hex(), app.finish.outline.to_hex());
        }
        assert!(!finishes.is_empty());
    }

    #[test]
    fn addresses_must_be_web_addresses() {
        assert!(fetch("10.0.0.38:3100/studio/api").unwrap_err().contains("http"));
    }
}
