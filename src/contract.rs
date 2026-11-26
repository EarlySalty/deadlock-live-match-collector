use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Number, Value};
use std::fmt;

const SCHEMA: &str = include_str!("../docs/contract/live-match-api-v3.schema.json");

struct Strict(Value);

impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(StrictVisitor)
    }
}

struct StrictVisitor;

impl<'de> Visitor<'de> for StrictVisitor {
    type Value = Strict;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("JSON without duplicate object keys")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Strict, E> {
        Ok(Strict(Value::Bool(value)))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Strict, E> {
        Ok(Strict(Value::from(value)))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Strict, E> {
        Ok(Strict(Value::from(value)))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Strict, E> {
        Number::from_f64(value)
            .map(|number| Strict(Value::Number(number)))
            .ok_or_else(|| E::custom("nonfinite number"))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Strict, E> {
        Ok(Strict(Value::from(value)))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Strict, E> {
        Ok(Strict(Value::Null))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Strict, A::Error> {
        let mut items = Vec::new();
        while let Some(Strict(item)) = seq.next_element()? {
            items.push(item);
        }
        Ok(Strict(Value::Array(items)))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Strict, A::Error> {
        let mut values = Map::new();
        while let Some((key, Strict(value))) = map.next_entry::<String, Strict>()? {
            if values.insert(key, value).is_some() {
                return Err(de::Error::custom("duplicate object key"));
            }
        }
        Ok(Strict(Value::Object(values)))
    }
}

fn reference<'a>(node: &Value, root: &'a Value) -> Option<&'a Value> {
    let path = node.get("$ref")?.as_str()?.strip_prefix('#')?;
    root.pointer(path)
}

fn kind(value: &Value, name: &str) -> bool {
    match name {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "integer" => value.is_i64() || value.is_u64(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    }
}

fn shape(value: &Value, node: &Value, root: &Value) -> bool {
    if let Some(target) = reference(node, root) {
        return shape(value, target, root);
    }
    if let Some(types) = node.get("type") {
        let valid = if let Some(types) = types.as_array() {
            types
                .iter()
                .any(|t| t.as_str().is_some_and(|t| kind(value, t)))
        } else {
            types.as_str().is_some_and(|t| kind(value, t))
        };
        if !valid {
            return false;
        }
    }
    if node
        .get("minimum")
        .and_then(Value::as_i64)
        .is_some_and(|n| n >= 0)
        && value.as_i64().is_some_and(|n| n < 0)
    {
        return false;
    }
    if let Some(options) = node.get("anyOf").and_then(Value::as_array) {
        if !options.iter().any(|option| shape(value, option, root)) {
            return false;
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = node.get("required").and_then(Value::as_array) {
            if !required
                .iter()
                .all(|key| key.as_str().is_some_and(|key| object.contains_key(key)))
            {
                return false;
            }
        }
        let properties = node.get("properties").and_then(Value::as_object);
        for (key, member) in object {
            if let Some(property) = properties.and_then(|props| props.get(key)) {
                if !shape(member, property, root) {
                    return false;
                }
            } else if node.get("additionalProperties") == Some(&Value::Bool(false)) {
                return false;
            }
        }
    }
    if let (Some(items), Some(item)) = (value.as_array(), node.get("items")) {
        if !items.iter().all(|value| shape(value, item, root)) {
            return false;
        }
    }
    true
}

fn canonical_writer(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            14 => b == b'4',
            19 => matches!(b, b'8' | b'9' | b'a' | b'b'),
            _ => b.is_ascii_digit() || (b'a'..=b'f').contains(&b),
        })
}

fn limits(length: usize, node: &Value, min: &str, max: &str) -> bool {
    node.get(min)
        .and_then(Value::as_u64)
        .is_none_or(|n| length as u64 >= n)
        && node
            .get(max)
            .and_then(Value::as_u64)
            .is_none_or(|n| length as u64 <= n)
}

fn matches_schema(value: &Value, node: &Value, root: &Value) -> bool {
    if !shape(value, node, root) {
        return false;
    }
    if let Some(target) = reference(node, root) {
        return matches_schema(value, target, root);
    }
    if node.get("const").is_some_and(|expected| value != expected)
        || node
            .get("enum")
            .and_then(Value::as_array)
            .is_some_and(|allowed| !allowed.contains(value))
    {
        return false;
    }
    if let Some(n) = value
        .as_i64()
        .map(i128::from)
        .or_else(|| value.as_u64().map(i128::from))
    {
        if node
            .get("minimum")
            .and_then(Value::as_i64)
            .is_some_and(|min| n < i128::from(min))
            || node
                .get("maximum")
                .and_then(Value::as_u64)
                .is_some_and(|max| n > i128::from(max))
        {
            return false;
        }
    }
    if let Some(s) = value.as_str() {
        if !limits(s.chars().count(), node, "minLength", "maxLength")
            || (node.get("pattern").is_some() && !canonical_writer(s))
        {
            return false;
        }
    }
    if let Some(items) = value.as_array() {
        if !limits(items.len(), node, "minItems", "maxItems") {
            return false;
        }
        if node.get("uniqueItems") == Some(&Value::Bool(true))
            && items
                .iter()
                .enumerate()
                .any(|(i, item)| items[..i].contains(item))
        {
            return false;
        }
        if let Some(item) = node.get("items") {
            if !items.iter().all(|value| matches_schema(value, item, root)) {
                return false;
            }
        }
        if let Some(contains) = node.get("contains") {
            let count = items
                .iter()
                .filter(|item| matches_schema(item, contains, root))
                .count() as u64;
            let minimum = node.get("minContains").and_then(Value::as_u64).unwrap_or(1);
            if count < minimum
                || node
                    .get("maxContains")
                    .and_then(Value::as_u64)
                    .is_some_and(|max| count > max)
            {
                return false;
            }
        }
    }
    if let (Some(object), Some(properties)) = (
        value.as_object(),
        node.get("properties").and_then(Value::as_object),
    ) {
        if !properties.iter().all(|(key, schema)| {
            object
                .get(key)
                .is_none_or(|member| matches_schema(member, schema, root))
        }) {
            return false;
        }
    }
    if let Some(options) = node.get("anyOf").and_then(Value::as_array) {
        if !options
            .iter()
            .any(|option| matches_schema(value, option, root))
        {
            return false;
        }
    }
    if let Some(rules) = node.get("allOf").and_then(Value::as_array) {
        if !rules.iter().all(|rule| matches_schema(value, rule, root)) {
            return false;
        }
    }
    if node
        .get("not")
        .is_some_and(|rule| matches_schema(value, rule, root))
    {
        return false;
    }
    if node
        .get("if")
        .is_some_and(|condition| matches_schema(value, condition, root))
        && node
            .get("then")
            .is_some_and(|rule| !matches_schema(value, rule, root))
    {
        return false;
    }
    true
}

pub fn body_status(body: &[u8]) -> u16 {
    if body.len() > 32_768 {
        return 413;
    }
    let Strict(value) = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(error) if error.is_data() => return 422,
        Err(_) => return 400,
    };
    let root: Value = serde_json::from_str(SCHEMA).expect("valid checked-in schema");
    if !shape(&value, &root, &root) {
        return 422;
    }
    let hero_valid = value["hero"].as_str().is_some_and(|name| {
        (1..=80).contains(&name.len()) && name.trim() == name && !name.chars().any(char::is_control)
    });
    if hero_valid && matches_schema(&value, &root, &root) {
        204
    } else {
        400
    }
}
