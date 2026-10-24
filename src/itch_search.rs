//! itch.io's own game search (`/search/games`), which butler does not
//! offer: its Search.Games only looks in the local database. The API
//! answers in snake_case and names platforms as traits; both are turned
//! into butler's shapes the way go-itchio does.
//!
//! The search answers one page of at most 50 games, mostly by title, so
//! browsing a type reads itch.io's browse pages instead and looks up each
//! game they list.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use crate::model::Game;

const SITE: &str = "https://itch.io";

/// The games matching `query`, in itch.io's order.
pub fn search(api_url: &str, api_key: &str, query: &str) -> Result<Vec<Game>> {
    let url = format!("{api_url}/search/games");
    let request = crate::http::agent()
        .get(&url)
        .query("query", query)
        .header("Authorization", &format!("Bearer {api_key}"));
    let json = fetch(request, &url)?;
    let Value::Object(mut root) = json else {
        return Ok(Vec::new());
    };
    let Some(Value::Array(games)) = root.remove("games") else {
        return Ok(Vec::new());
    };
    Ok(games.into_iter().filter_map(game_from).collect())
}

/// The ids on one page of itch.io's browse listing at `path`, like
/// `tag-nes`, in its order; none past the last page.
pub fn browse_page(path: &str, page: u32) -> Result<Vec<i64>> {
    let url = format!("{SITE}/games/{path}");
    let request = crate::http::agent()
        .get(&url)
        .query("format", "json")
        .query("page", page.to_string());
    let json = fetch(request, &url)?;
    Ok(game_ids(json.get("content").and_then(Value::as_str).unwrap_or("")))
}

/// One game with its scanned platforms, which browse pages leave out.
/// itch.io turns away more than a couple of these at once, so a refusal
/// waits and tries again.
pub fn game(api_url: &str, api_key: &str, id: i64) -> Result<Option<Game>> {
    let url = format!("{api_url}/games/{id}");
    let mut tries = 0;
    loop {
        let request = crate::http::agent()
            .get(&url)
            .header("Authorization", &format!("Bearer {api_key}"));
        match fetch(request, &url) {
            Ok(mut json) => {
                return Ok(json.get_mut("game").map(Value::take).and_then(game_from));
            }
            Err(error) if tries < 3 && error.to_string().starts_with("429") => {
                tries += 1;
                std::thread::sleep(Duration::from_secs(tries));
            }
            Err(error) => return Err(error),
        }
    }
}

fn fetch(
    request: ureq::RequestBuilder<ureq::typestate::WithoutBody>,
    url: &str,
) -> Result<Value> {
    let mut response = request
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .with_context(|| format!("fetching {url}"))?;
    let status = response.status();
    let body = response.body_mut().read_to_string().unwrap_or_default();
    if !status.is_success() {
        bail!(
            "{}: {}",
            status.as_u16(),
            body.trim().chars().take(300).collect::<String>()
        );
    }
    serde_json::from_str(&body).with_context(|| format!("reading {url}"))
}

fn game_ids(html: &str) -> Vec<i64> {
    let mut ids: Vec<i64> = Vec::new();
    for part in html.split("data-game_id=\"").skip(1) {
        let Some(id) = part.split('"').next().and_then(|id| id.parse().ok()) else {
            continue;
        };
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

fn game_from(game: Value) -> Option<Game> {
    let mut game = untrait(camelify(game));
    if let Value::Object(map) = &mut game {
        map.remove("user");
    }
    match serde_json::from_value::<Game>(game.clone()) {
        Ok(parsed) => Some(parsed),
        Err(first) => {
            // A date in a form butler's type does not take should
            // not cost the whole game.
            if let Value::Object(map) = &mut game {
                map.remove("createdAt");
                map.remove("publishedAt");
            }
            serde_json::from_value(game)
                .map_err(|_| log::warn!("skipping a search result: {first}"))
                .ok()
        }
    }
}

/// Keys go-itchio leaves as they are.
const KEEP: [&str; 3] = ["upload_headers", "launch_targets", "cookie"];

fn camelify(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let value = if KEEP.contains(&key.as_str()) {
                        value
                    } else {
                        camelify(value)
                    };
                    (camelcase(&key), value)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(camelify).collect()),
        other => other,
    }
}

fn camelcase(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    let mut upper = false;
    for c in key.chars() {
        if c == '_' {
            upper = !out.is_empty();
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Turns the API's `traits` into butler's platforms and flags.
fn untrait(game: Value) -> Value {
    let Value::Object(mut map) = game else {
        return game;
    };
    let Some(Value::Array(traits)) = map.remove("traits") else {
        return Value::Object(map);
    };
    let mut platforms = Map::new();
    for name in traits.iter().filter_map(Value::as_str) {
        match name {
            "p_windows" => {
                platforms.insert("windows".into(), "all".into());
            }
            "p_linux" => {
                platforms.insert("linux".into(), "all".into());
            }
            "p_osx" => {
                platforms.insert("osx".into(), "all".into());
            }
            "can_be_bought" => {
                map.insert("canBeBought".into(), true.into());
            }
            "has_demo" => {
                map.insert("hasDemo".into(), true.into());
            }
            "in_press_system" => {
                map.insert("inPressSystem".into(), true.into());
            }
            _ => {}
        }
    }
    map.insert("platforms".into(), Value::Object(platforms));
    Value::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_become_camel_case() {
        assert_eq!(camelcase("cover_url"), "coverUrl");
        assert_eq!(camelcase("min_price"), "minPrice");
        assert_eq!(camelcase("id"), "id");
    }

    #[test]
    fn results_read_as_games() {
        let json = serde_json::json!({
            "page": 1,
            "games": [{
                "id": 7,
                "title": "Squatch Quest",
                "url": "https://x.itch.io/squatch",
                "cover_url": "https://img/c.png",
                "min_price": 0,
                "traits": ["p_linux", "has_demo"]
            }]
        });
        let games: Vec<Game> = json["games"]
            .as_array()
            .unwrap()
            .iter()
            .cloned()
            .filter_map(game_from)
            .collect();
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].id, 7);
        assert_eq!(games[0].cover_url.as_deref(), Some("https://img/c.png"));
        assert_eq!(games[0].has_demo, Some(true));
        assert!(games[0].platforms.linux.is_some());
    }

    #[test]
    fn browse_ids_in_order_once() {
        let html = r#"<div data-game_id="459250"><a data-game_id="459250"></a></div><div data-game_id="336785">"#;
        assert_eq!(game_ids(html), vec![459250, 336785]);
    }
}
