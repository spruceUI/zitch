//! itch.io's own game search (`/search/games`), which butler does not
//! offer: its Search.Games only looks in the local database. The API
//! answers in snake_case and names platforms as traits; both are turned
//! into butler's shapes the way go-itchio does.

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use crate::model::Game;

/// One page of games matching `query`, in itch.io's order.
pub fn search(api_url: &str, api_key: &str, query: &str, page: u32) -> Result<Vec<Game>> {
    let url = format!("{api_url}/search/games");
    let mut response = crate::http::agent()
        .get(&url)
        .query("query", query)
        .query("page", page.to_string())
        .header("Authorization", &format!("Bearer {api_key}"))
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .with_context(|| format!("searching {url}"))?;
    let status = response.status();
    let body = response.body_mut().read_to_string().unwrap_or_default();
    if !status.is_success() {
        bail!("{status}: {}", body.trim().chars().take(300).collect::<String>());
    }
    let json: Value = serde_json::from_str(&body).context("reading the search results")?;
    Ok(games(json))
}

fn games(json: Value) -> Vec<Game> {
    let Value::Object(mut root) = json else {
        return Vec::new();
    };
    let Some(Value::Array(games)) = root.remove("games") else {
        return Vec::new();
    };
    games
        .into_iter()
        .filter_map(|game| {
            let mut game = untrait(camelify(game));
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
        })
        .collect()
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
        let games = games(json);
        assert_eq!(games.len(), 1);
        assert_eq!(games[0].id, 7);
        assert_eq!(games[0].cover_url.as_deref(), Some("https://img/c.png"));
        assert_eq!(games[0].has_demo, Some(true));
        assert!(games[0].platforms.linux.is_some());
    }
}
