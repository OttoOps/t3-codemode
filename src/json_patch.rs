use serde_json::{Map, Value};

struct Style {
    newline: &'static str,
    unit: String,
}

#[derive(Clone, Copy)]
struct Item<'a> {
    lead: &'a str,
    head: &'a str,
    value: &'a str,
    trail: &'a str,
}

impl Item<'_> {
    fn name(&self) -> Option<String> {
        let end = string_end(self.head, 0);
        serde_json::from_str(&self.head[..end]).ok()
    }
}

pub(crate) fn patch(text: &str, old: &Value, new: &Value) -> String {
    let start = skip_space(text, 0);
    let end = value_end(text, start);
    let raw = &text[start..end];
    let style = Style { newline: if text.contains("\r\n") { "\r\n" } else { "\n" }, unit: indent_unit(raw) };
    format!("{}{}{}", &text[..start], patch_value(raw, old, new, Some(""), &style), &text[end..])
}

fn indent_unit(raw: &str) -> String {
    raw.lines()
        .skip(1)
        .map(|line| &line[..line.len() - line.trim_start_matches([' ', '\t']).len()])
        .find(|indent| !indent.is_empty())
        .unwrap_or("  ")
        .to_string()
}

fn patch_value(raw: &str, old: &Value, new: &Value, indent: Option<&str>, style: &Style) -> String {
    if old == new {
        return raw.to_string();
    }
    match (old, new) {
        (Value::Object(o), Value::Object(n)) if !o.is_empty() && !n.is_empty() => patch_object(raw, o, n, style),
        (Value::Array(o), Value::Array(n)) if !o.is_empty() && !n.is_empty() => patch_array(raw, o, n, style),
        _ => render(new, indent, style),
    }
}

fn patch_object(raw: &str, old: &Map<String, Value>, new: &Map<String, Value>, style: &Style) -> String {
    let (items, tail) = items(raw, true);
    let names: Vec<String> = items.iter().map(|i| i.name().unwrap_or_default()).collect();
    let sep = separator(&items);
    let indent = line_indent(sep);
    let mut out = Vec::new();
    for (i, (item, name)) in items.iter().zip(&names).enumerate() {
        let (Some(before), Some(after)) = (old.get(name), new.get(name)) else { continue };
        let body = if before == after || names[i + 1..].contains(name) {
            item.value.to_string()
        } else {
            patch_value(item.value, before, after, indent, style)
        };
        out.push((Some(i), format!("{}{body}", item.head)));
    }
    let colon = if items.last().is_some_and(|i| i.head.ends_with(':')) { ":" } else { ": " };
    for (name, value) in new.iter().filter(|(name, _)| !old.contains_key(*name)) {
        out.push((None, format!("{}{colon}{}", Value::from(name.as_str()), render(value, indent, style))));
    }
    assemble(('{', '}'), &items, tail, &out)
}

fn patch_array(raw: &str, old: &[Value], new: &[Value], style: &Style) -> String {
    let (items, tail) = items(raw, false);
    let sep = separator(&items);
    let indent = line_indent(sep);
    let added_indent = indent.filter(|_| items.iter().any(|i| i.value.contains('\n')));
    let mut out = Vec::new();
    let mut from = 0;
    for (i, value) in new.iter().enumerate() {
        if old.len() == new.len() {
            out.push((Some(i), patch_value(items[i].value, &old[i], value, indent, style)));
        } else if let Some(found) = old[from..].iter().position(|o| o == value) {
            out.push((Some(from + found), items[from + found].value.to_string()));
            from += found + 1;
        } else {
            out.push((None, render(value, added_indent, style)));
        }
    }
    assemble(('[', ']'), &items, tail, &out)
}

fn assemble((open, close): (char, char), items: &[Item], tail: &str, out: &[(Option<usize>, String)]) -> String {
    let mut text = String::from(open);
    for (pos, (source, body)) in out.iter().enumerate() {
        text.push_str(match source {
            _ if pos == 0 => items[0].lead,
            Some(i) if *i > 0 => items[*i].lead,
            _ => separator(items),
        });
        text.push_str(body);
        if pos + 1 < out.len() {
            text.push_str(source.map_or("", |i| items[i].trail));
            text.push(',');
        }
    }
    text.push_str(tail);
    text.push(close);
    text
}

fn separator<'a>(items: &[Item<'a>]) -> &'a str {
    match items {
        [only] if only.lead.contains('\n') => only.lead,
        [_] | [] => " ",
        [.., last] => last.lead,
    }
}

fn line_indent(lead: &str) -> Option<&str> {
    lead.rfind('\n').map(|at| &lead[at + 1..])
}

fn render(value: &Value, indent: Option<&str>, style: &Style) -> String {
    let pretty = format!("{value:#}");
    let Some(indent) = indent else {
        return pretty.lines().map(str::trim_start).collect::<Vec<_>>().join(" ");
    };
    let mut out = String::new();
    for (i, line) in pretty.lines().enumerate() {
        let body = line.trim_start_matches(' ');
        if i > 0 {
            out.push_str(style.newline);
            out.push_str(indent);
            out.push_str(&style.unit.repeat((line.len() - body.len()) / 2));
        }
        out.push_str(body);
    }
    out
}

fn items(raw: &str, object: bool) -> (Vec<Item<'_>>, &str) {
    let mut out: Vec<Item> = Vec::new();
    let mut at = 1;
    loop {
        let start = skip_space(raw, at);
        if start + 1 >= raw.len() {
            return (out, &raw[at..start]);
        }
        let value_start = if object {
            let colon = skip_space(raw, string_end(raw, start));
            skip_space(raw, colon + 1)
        } else {
            start
        };
        let end = value_end(raw, value_start);
        let next = skip_space(raw, end);
        let more = raw.as_bytes().get(next) == Some(&b',');
        out.push(Item {
            lead: &raw[at..start],
            head: &raw[start..value_start],
            value: &raw[value_start..end],
            trail: if more { &raw[end..next] } else { "" },
        });
        if !more {
            return (out, &raw[end..next]);
        }
        at = next + 1;
    }
}

fn skip_space(text: &str, from: usize) -> usize {
    text.bytes().skip(from).position(|b| !b.is_ascii_whitespace()).map_or(text.len(), |n| from + n)
}

fn string_end(text: &str, from: usize) -> usize {
    let mut escaped = false;
    for (i, b) in text.bytes().enumerate().skip(from + 1) {
        match b {
            _ if escaped => escaped = false,
            b'\\' => escaped = true,
            b'"' => return i + 1,
            _ => {}
        }
    }
    text.len()
}

fn value_end(text: &str, from: usize) -> usize {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut at = from;
    while let Some(b) = bytes.get(at) {
        match b {
            b'"' => {
                at = string_end(text, at);
                if depth == 0 {
                    return at;
                }
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' if depth == 0 => return at,
            b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return at + 1;
                }
            }
            b',' if depth == 0 => return at,
            b if depth == 0 && b.is_ascii_whitespace() => return at,
            _ => {}
        }
        at += 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn patched(text: &str, change: impl FnOnce(&mut Value)) -> String {
        let old: Value = serde_json::from_str(text).unwrap();
        let mut new = old.clone();
        change(&mut new);
        let out = patch(text, &old, &new);
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap(), new, "{out}");
        out
    }

    #[test]
    fn a_changed_value_leaves_every_other_byte_alone() {
        let text = "{\n    \"a\" :  1.50,\n\n    \"b\": [\"x\",\"y\"],\n    \"c\": \"\\\"},\"\n}\n";
        assert_eq!(patched(text, |v| v["b"][1] = json!("z")), text.replace("\"y\"", "\"z\""));
    }

    #[test]
    fn new_keys_follow_the_files_indent_and_newlines() {
        let text = "{\r\n\t\"a\": 1\r\n}\r\n";
        assert_eq!(
            patched(text, |v| v["b"] = json!({"c": 2})),
            "{\r\n\t\"a\": 1,\r\n\t\"b\": {\r\n\t\t\"c\": 2\r\n\t}\r\n}\r\n"
        );
    }

    #[test]
    fn one_line_containers_stay_on_one_line() {
        assert_eq!(patched(r#"{"a":1}"#, |v| v["b"] = json!(2)), r#"{"a":1, "b":2}"#);
        let text = "{\n  \"m\": [\n    { \"id\": \"a\" },\n    { \"id\": \"b\" }\n  ]\n}\n";
        let grown = patched(text, |v| v["m"].as_array_mut().unwrap().push(json!({"id": "c"})));
        assert_eq!(grown, text.replace("\"b\" }", "\"b\" },\n    { \"id\": \"c\" }"));
    }

    #[test]
    fn removing_the_first_middle_or_last_item_keeps_the_rest() {
        let text = "{\n  \"a\": [1, 2, 3],\n  \"b\": true,\n  \"c\": null\n}\n";
        for (i, want) in ["[2, 3]", "[1, 3]", "[1, 2]"].iter().enumerate() {
            let out = patched(text, |v| {
                v["a"].as_array_mut().unwrap().remove(i);
            });
            assert_eq!(out, text.replace("[1, 2, 3]", want));
        }
        for (key, line) in [("a", "\n  \"a\": [1, 2, 3],"), ("b", "\n  \"b\": true,"), ("c", ",\n  \"c\": null")] {
            let out = patched(text, |v| {
                v.as_object_mut().unwrap().shift_remove(key);
            });
            assert_eq!(out, text.replace(line, ""));
        }
    }

    #[test]
    fn containers_can_empty_and_fill() {
        assert_eq!(patched("{\"a\": [1]}", |v| v["a"] = json!([])), "{\"a\": []}");
        assert_eq!(patched("{}\n", |v| v["a"] = json!({"b": 1})), "{\n  \"a\": {\n    \"b\": 1\n  }\n}\n");
        assert_eq!(patched("{\n  \"a\": {}\n}", |v| v["a"]["b"] = json!(1)), "{\n  \"a\": {\n    \"b\": 1\n  }\n}");
    }

    #[test]
    fn a_repeated_key_is_edited_where_it_counts_and_removed_everywhere() {
        let text = r#"{"a": 1, "b": 0, "a": 2}"#;
        assert_eq!(patched(text, |v| v["a"] = json!(3)), r#"{"a": 1, "b": 0, "a": 3}"#);
        assert_eq!(
            patched(text, |v| {
                v.as_object_mut().unwrap().shift_remove("a");
            }),
            r#"{"b": 0}"#
        );
    }
}
