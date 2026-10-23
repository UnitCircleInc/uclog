//! CBOR helpers over `ciborium::Value`.

pub use ciborium::Value;

pub fn from_slice(data: &[u8]) -> Result<Value, ciborium::de::Error<std::io::Error>> {
    ciborium::from_reader(data)
}

pub fn to_vec(v: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(v, &mut out).expect("writing CBOR to a Vec cannot fail");
    out
}

/// Look up a text key in a CBOR map.
pub fn get<'a>(map: &'a Value, key: &str) -> Option<&'a Value> {
    map.as_map()?.iter().find(|(k, _)| k.as_text() == Some(key)).map(|(_, v)| v)
}

pub fn get_text<'a>(map: &'a Value, key: &str) -> Option<&'a str> {
    get(map, key)?.as_text()
}

pub fn get_bytes<'a>(map: &'a Value, key: &str) -> Option<&'a [u8]> {
    get(map, key)?.as_bytes().map(|b| b.as_slice())
}

pub fn get_int(map: &Value, key: &str) -> Option<i128> {
    get(map, key)?.as_integer().map(i128::from)
}

/// Build a CBOR map with text keys.
pub fn map<I: IntoIterator<Item = (&'static str, Value)>>(items: I) -> Value {
    Value::Map(items.into_iter().map(|(k, v)| (Value::Text(k.into()), v)).collect())
}

/// Render a CBOR value as compact JSON-like text for display.
pub fn display(v: &Value) -> String {
    match v {
        Value::Integer(i) => i128::from(*i).to_string(),
        Value::Bytes(b) => format!("h'{}'", b.iter().map(|x| format!("{x:02x}")).collect::<String>()),
        Value::Float(f) => f.to_string(),
        Value::Text(s) => format!("{s:?}"),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".into(),
        Value::Tag(t, v) => format!("{t}({})", display(v)),
        Value::Array(a) => format!("[{}]", a.iter().map(display).collect::<Vec<_>>().join(", ")),
        Value::Map(m) => format!(
            "{{{}}}",
            m.iter().map(|(k, v)| format!("{}: {}", display(k), display(v))).collect::<Vec<_>>().join(", ")
        ),
        _ => "?".into(),
    }
}
