#[path = "../src/contract.rs"]
mod contract;

use serde_json::{json, Value};

const VECTORS: &str = include_str!("../docs/contract/live-match-api-v3-vectors.json");
const SCHEMA: &str = include_str!("../docs/contract/live-match-api-v3.schema.json");

fn vectors() -> Value {
    serde_json::from_str(VECTORS).unwrap()
}

fn fixture(name: &str) -> Value {
    vectors()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap()["body"]
        .clone()
}

fn status(value: &Value) -> u16 {
    contract::body_status(&serde_json::to_vec(value).unwrap())
}

#[test]
fn agreed_vectors_match_independent_body_oracle() {
    let vectors = vectors();
    assert_eq!(vectors["api_version"], 3);
    assert_eq!(vectors["schema_version"], 3);
    assert_eq!(vectors["upload_path"], "/twitch/api/v3/live-match");
    assert_eq!(vectors["max_body_bytes"], 32768);
    let cases = vectors["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 39);
    for case in cases {
        assert_eq!(
            status(&case["body"]),
            case["status"].as_u64().unwrap() as u16,
            "{}",
            case["name"]
        );
    }
}

#[test]
fn captured_synthetic_client_bytes_pass_the_independent_oracle() {
    let bytes = include_bytes!("fixtures/client-schema3-synthetic.json");
    assert_eq!(bytes.len(), 9112);
    assert_eq!(contract::body_status(bytes), 204);
    let value: Value = serde_json::from_slice(bytes).unwrap();
    for field in [
        "kills",
        "deaths",
        "assists",
        "team_souls",
        "enemy_souls",
        "game_mode",
        "midboss",
    ] {
        assert!(value.get(field).is_some(), "{field}");
    }
    for section in [
        "scoreboard",
        "teams",
        "own_loadout",
        "structures",
        "objectives",
    ] {
        assert!(value["details"].get(section).is_some(), "{section}");
    }
}

#[test]
fn full_fixture_exercises_original_functional_sections() {
    let value = fixture("full-original-fields");
    assert_eq!(status(&value), 204);
    assert_eq!(value["kills"], 2);
    assert_eq!(value["deaths"], 3);
    assert_eq!(value["assists"], 4);
    assert_eq!(value["team_souls"], 100000);
    assert_eq!(value["enemy_souls"], 95000);
    assert_eq!(value["game_mode"], "normal");
    assert_eq!(value["details"]["scoreboard"].as_array().unwrap().len(), 12);
    assert_eq!(value["details"]["teams"].as_array().unwrap().len(), 2);
    assert_eq!(value["details"]["own_loadout"]["items"], json!([123, 456]));
    assert_eq!(
        value["details"]["own_loadout"]["ability_upgrades"][0]["points"],
        2
    );
    assert_eq!(value["details"]["structures"].as_array().unwrap().len(), 2);
    assert_eq!(
        value["details"]["objectives"]["rift"]["next_spawn_in_seconds"],
        30
    );
    assert_eq!(
        value["details"]["objectives"]["bridge_buffs"]["estimated_spawn_in_seconds"],
        120
    );
    assert_eq!(
        value["details"]["objectives"]["urn"]["present"],
        Value::Null
    );
    let bytes = serde_json::to_vec(&value).unwrap();
    assert_eq!(bytes.len(), 4412);
    let maximal = fixture("maximal-shape");
    let max_bytes = serde_json::to_vec(&maximal).unwrap();
    assert_eq!(max_bytes.len(), 10084);
    assert_eq!(contract::body_status(&max_bytes), 204);
    assert_eq!(
        maximal["details"]["structures"].as_array().unwrap().len(),
        48
    );
    assert_eq!(
        maximal["details"]["own_loadout"]["items"]
            .as_array()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(
        maximal["details"]["own_loadout"]["ability_upgrades"]
            .as_array()
            .unwrap()
            .len(),
        16
    );
}

#[test]
fn encoded_bytes_include_unicode_escaping_and_padding() {
    let mut value = fixture("full-original-fields");
    value["hero"] = json!(format!("{}\\\"", "ä".repeat(39)));
    let bytes = serde_json::to_vec(&value).unwrap();
    assert_eq!(value["hero"].as_str().unwrap().len(), 80);
    assert_eq!(contract::body_status(&bytes), 204);
    for (size, expected) in [(32767, 204), (32768, 204), (32769, 413)] {
        let mut body = bytes.clone();
        body.resize(size, b' ');
        assert_eq!(contract::body_status(&body), expected);
    }
    value["hero"] = json!("ä".repeat(41));
    assert_eq!(status(&value), 400);
}

#[test]
fn documented_schema_is_closed_and_versioned() {
    let schema: Value = serde_json::from_str(SCHEMA).unwrap();
    assert_eq!(schema["properties"]["schema_version"]["const"], 3);
    assert_eq!(schema["additionalProperties"], false);
    for name in [
        "details",
        "scoreboard",
        "team",
        "loadout",
        "ability",
        "structure",
        "objectives",
        "objective",
        "midboss",
    ] {
        assert_eq!(
            schema["$defs"][name]["additionalProperties"], false,
            "{name}"
        );
    }
    assert_eq!(
        schema["$defs"]["details"]["properties"]["schema_version"]["const"],
        1
    );
    assert_eq!(schema["required"].as_array().unwrap().len(), 6);
}

#[test]
fn hero_and_writer_require_canonical_text() {
    let mut value = fixture("minimal");
    for hero in [
        "",
        " Hero",
        "Hero ",
        "\u{a0}Hero",
        "Hero\u{85}X",
        "Hero\u{7f}X",
        "Hero\0X",
    ] {
        value["hero"] = json!(hero);
        assert_eq!(status(&value), 400, "{hero:?}");
    }
    value["hero"] = json!("英雄");
    assert_eq!(status(&value), 204);
    for writer in [
        "123E4567-e89b-42d3-a456-426614174000",
        "123e4567-e89b-12d3-a456-426614174000",
        "123e4567-e89b-42d3-7456-426614174000",
        "123e4567-e89b-42d3-a456-426614174000\n",
    ] {
        value["writer_id"] = json!(writer);
        assert_eq!(status(&value), 400);
    }
}

#[test]
fn forbidden_identifiers_and_hidden_fields_are_rejected_at_every_depth() {
    let full = fixture("full-original-fields");
    for path in [
        "",
        "/details",
        "/details/scoreboard/0",
        "/details/teams/0",
        "/details/own_loadout",
        "/details/structures/0",
        "/details/objectives/urn",
        "/midboss",
    ] {
        for field in [
            "match_id",
            "name",
            "steam_id",
            "raw_memory",
            "position",
            "ultimate_cooldown",
            "process_address",
            "key",
        ] {
            let mut value = full.clone();
            value.pointer_mut(path).unwrap()[field] = Value::Null;
            assert_eq!(status(&value), 422, "{path}/{field}");
        }
    }
}

#[test]
fn missing_values_are_not_fabricated_and_invalid_ranges_are_rejected() {
    let mut value = fixture("unknown-summary-and-detail");
    assert_eq!(status(&value), 204);
    assert_eq!(value["kills"], Value::Null);
    value["sequence"] = json!(9_007_199_254_740_991_u64);
    value["match_epoch"] = json!(9_007_199_254_740_991_u64);
    value["game_seconds"] = json!(86400);
    assert_eq!(status(&value), 204);
    value["match_epoch"] = json!(9_007_199_254_740_992_u64);
    assert_eq!(status(&value), 400);
    let mut full = fixture("full-original-fields");
    full["details"]["teams"][0]["souls"] = Value::Null;
    full["details"]["teams"][0]["complete"] = json!(false);
    full["details"]["own_loadout"]["health"] = json!(-1);
    assert_eq!(status(&full), 204);
    full["details"]["own_loadout"]["health"] = json!(-2);
    assert_eq!(status(&full), 400);
}

#[test]
fn malformed_and_structurally_invalid_bodies_are_distinct() {
    assert_eq!(contract::body_status(b"{"), 400);
    assert_eq!(contract::body_status(b"null"), 422);
    assert_eq!(
        contract::body_status(br#"[3,"123e4567-e89b-42d3-a456-426614174000",1,1,"Infernus",60]"#),
        422
    );
    let duplicate = br#"{"schema_version":3,"schema_version":3,"writer_id":"123e4567-e89b-42d3-a456-426614174000","sequence":1,"match_epoch":1,"hero":"Infernus","game_seconds":60}"#;
    assert_eq!(contract::body_status(duplicate), 422);
    let mut value = fixture("full-original-fields");
    value["details"]["scoreboard"][0] = json!(["own", true]);
    assert_eq!(status(&value), 422);
    let mut value = fixture("minimal");
    value["paused"] = json!("true");
    assert_eq!(status(&value), 422);
    value["paused"] = Value::Null;
    value["sequence"] = json!(1.5);
    assert_eq!(status(&value), 422);
}
