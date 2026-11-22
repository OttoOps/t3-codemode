use anyhow::{Result, bail};

#[derive(Debug)]
pub struct Invalid {
    pub field: String,
    message: String,
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Invalid {}

pub fn invalid(field: impl Into<String>, message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Invalid { field: field.into(), message: message.into() })
}

pub fn of(error: &anyhow::Error) -> &str {
    error.downcast_ref::<Invalid>().map_or("", |i| i.field.as_str())
}

pub fn within(error: anyhow::Error, field: &str, label: &str) -> anyhow::Error {
    invalid(join(field, of(&error)), format!("{label}: {error:#}"))
}

pub fn segment(name: &str) -> String {
    if name.is_empty() || name.contains(['.', '"']) {
        serde_json::Value::from(name).to_string()
    } else {
        name.to_string()
    }
}

pub fn join(prefix: &str, rest: &str) -> String {
    match (prefix, rest) {
        ("", only) | (only, "") => only.to_string(),
        _ => format!("{prefix}.{rest}"),
    }
}

pub fn display<S: AsRef<str>>(path: &[S]) -> String {
    path.iter().map(|s| segment(s.as_ref())).collect::<Vec<_>>().join(".")
}

pub fn parse(raw: &str) -> Result<Vec<String>> {
    let mut path = Vec::new();
    let mut rest = raw;
    loop {
        let len = if rest.starts_with('"') {
            let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<String>();
            let Some(Ok(name)) = stream.next() else {
                bail!("{raw:?} is not a config path: a quoted name must be a JSON string");
            };
            path.push(name);
            stream.byte_offset()
        } else {
            let len = rest.find('.').unwrap_or(rest.len());
            let name = &rest[..len];
            if name.is_empty() || name.contains('"') {
                bail!("{raw:?} is not a config path: write dotted names, such as board.feed");
            }
            path.push(name.to_string());
            len
        };
        if len == rest.len() {
            return Ok(path);
        }
        let Some(next) = rest[len..].strip_prefix('.') else {
            bail!("{raw:?} is not a config path: expected a dot after a quoted name");
        };
        rest = next;
    }
}

pub fn deserialize<T: serde::de::DeserializeOwned>(
    value: serde_json::Value,
) -> std::result::Result<T, (String, serde_json::Error)> {
    serde_path_to_error::deserialize(value).map_err(|e| {
        use serde_path_to_error::Segment;
        let mut path: Vec<String> = e
            .path()
            .iter()
            .filter_map(|s| match s {
                Segment::Seq { index } => Some(index.to_string()),
                Segment::Map { key } => Some(key.clone()),
                Segment::Enum { .. } | Segment::Unknown => None,
            })
            .collect();
        let inner = e.into_inner();
        if let Some(name) = inner.to_string().strip_prefix("missing field `").and_then(|m| m.split('`').next()) {
            path.push(name.to_string());
        }
        (display(&path), inner)
    })
}
