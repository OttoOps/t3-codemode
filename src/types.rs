use serde_json::Value;

const MAX_DEPTH: usize = 10;

const RESERVED: &[&str] = &[
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "new",
    "null",
    "return",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
    "let",
    "static",
    "implements",
    "interface",
    "package",
    "private",
    "protected",
    "public",
    "await",
];

pub fn sanitize_identifier(name: &str) -> String {
    let mut out: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '$' { c } else { '_' }).collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if RESERVED.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

fn is_identifier(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

fn property_key(name: &str) -> String {
    if is_identifier(name) { name.to_string() } else { Value::String(name.to_string()).to_string() }
}

fn doc_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").replace("*/", "*\\/")
}

pub fn doc_block(text: &str, indent: &str) -> String {
    let lines: Vec<String> =
        text.lines().map(|l| l.trim_end().replace("*/", "*\\/")).filter(|l| !l.trim().is_empty()).collect();
    match lines.len() {
        0 => String::new(),
        1 => format!("{indent}/** {} */\n", lines[0].trim()),
        _ => {
            let mut s = format!("{indent}/**\n");
            for l in lines {
                s.push_str(&format!("{indent} * {}\n", l.trim()));
            }
            s.push_str(&format!("{indent} */\n"));
            s
        }
    }
}

struct Gen<'a> {
    root: &'a Value,
    refs: Vec<String>,
}

fn union(parts: Vec<String>) -> String {
    let mut seen: Vec<String> = Vec::new();
    for p in parts {
        if !seen.contains(&p) {
            seen.push(p);
        }
    }
    if seen.iter().any(|p| p == "unknown") {
        return "unknown".into();
    }
    seen.join(" | ")
}

fn needs_parens(t: &str) -> bool {
    let mut depth = 0i32;
    for c in t.chars() {
        match c {
            '{' | '(' | '[' | '<' => depth += 1,
            '}' | ')' | ']' | '>' => depth -= 1,
            '|' | '&' if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

impl<'a> Gen<'a> {
    fn resolve(&self, r: &str) -> Option<&'a Value> {
        let path = r.strip_prefix("#/")?;
        let mut cur = self.root;
        for part in path.split('/') {
            let part = part.replace("~1", "/").replace("~0", "~");
            cur = cur.get(&part)?;
        }
        Some(cur)
    }

    fn ts(&mut self, s: &Value, depth: usize, indent: usize) -> String {
        if depth > MAX_DEPTH {
            return "unknown".into();
        }
        let Some(obj) = s.as_object() else {
            return if s == &Value::Bool(false) { "never".into() } else { "unknown".into() };
        };
        if let Some(r) = obj.get("$ref").and_then(Value::as_str) {
            if self.refs.iter().any(|x| x == r) {
                return "unknown".into();
            }
            let Some(target) = self.resolve(r) else { return "unknown".into() };
            self.refs.push(r.to_string());
            let t = self.ts(target, depth + 1, indent);
            self.refs.pop();
            return t;
        }
        let mut t = self.ts_inner(obj, depth, indent);
        if obj.get("nullable") == Some(&Value::Bool(true)) && t != "unknown" {
            t = union(vec![t, "null".into()]);
        }
        t
    }

    fn ts_inner(&mut self, obj: &serde_json::Map<String, Value>, depth: usize, indent: usize) -> String {
        if let Some(c) = obj.get("const") {
            return c.to_string();
        }
        if let Some(e) = obj.get("enum").and_then(Value::as_array) {
            return union(e.iter().map(Value::to_string).collect());
        }
        for key in ["anyOf", "oneOf"] {
            if let Some(alts) = obj.get(key).and_then(Value::as_array) {
                return union(alts.iter().map(|a| self.ts(a, depth + 1, indent)).collect());
            }
        }
        if let Some(all) = obj.get("allOf").and_then(Value::as_array) {
            let parts: Vec<String> = all
                .iter()
                .map(|a| self.ts(a, depth + 1, indent))
                .filter(|t| t != "unknown")
                .map(|t| if needs_parens(&t) { format!("({t})") } else { t })
                .collect();
            return if parts.is_empty() { "unknown".into() } else { parts.join(" & ") };
        }
        match obj.get("type") {
            Some(Value::Array(types)) => {
                union(types.iter().filter_map(Value::as_str).map(|t| self.typed(t, obj, depth, indent)).collect())
            }
            Some(Value::String(t)) => self.typed(t, obj, depth, indent),
            _ if obj.contains_key("properties") => self.typed("object", obj, depth, indent),
            _ if obj.contains_key("items") => self.typed("array", obj, depth, indent),
            _ => "unknown".into(),
        }
    }

    fn typed(&mut self, t: &str, obj: &serde_json::Map<String, Value>, depth: usize, indent: usize) -> String {
        match t {
            "string" => "string".into(),
            "number" | "integer" => "number".into(),
            "boolean" => "boolean".into(),
            "null" => "null".into(),
            "array" => match obj.get("items").or_else(|| obj.get("prefixItems")) {
                Some(Value::Array(tuple)) => {
                    let parts: Vec<String> = tuple.iter().map(|i| self.ts(i, depth + 1, indent)).collect();
                    format!("[{}]", parts.join(", "))
                }
                Some(items) => {
                    let inner = self.ts(items, depth + 1, indent);
                    if needs_parens(&inner) { format!("({inner})[]") } else { format!("{inner}[]") }
                }
                None => "unknown[]".into(),
            },
            "object" => self.object(obj, depth, indent),
            _ => "unknown".into(),
        }
    }

    fn object(&mut self, obj: &serde_json::Map<String, Value>, depth: usize, indent: usize) -> String {
        let props = obj.get("properties").and_then(Value::as_object);
        let extra = obj.get("additionalProperties");
        let Some(props) = props.filter(|p| !p.is_empty()) else {
            return match extra {
                Some(Value::Bool(false)) => "{}".into(),
                Some(s @ Value::Object(_)) => format!("Record<string, {}>", self.ts(s, depth + 1, indent)),
                _ => "Record<string, unknown>".into(),
            };
        };
        let required: Vec<&str> = obj
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let pad = "  ".repeat(indent + 1);
        let mut s = String::from("{\n");
        for (name, schema) in props {
            if let Some(d) = schema.get("description").and_then(Value::as_str) {
                s.push_str(&format!("{pad}/** {} */\n", doc_line(d)));
            }
            let opt = if required.contains(&name.as_str()) { "" } else { "?" };
            let t = self.ts(schema, depth + 1, indent + 1);
            s.push_str(&format!("{pad}{}{opt}: {t};\n", property_key(name)));
        }
        if let Some(e @ Value::Object(_)) = extra {
            let t = self.ts(e, depth + 1, indent + 1);
            s.push_str(&format!("{pad}[key: string]: {t};\n"));
        }
        s.push_str(&"  ".repeat(indent));
        s.push('}');
        s
    }
}

pub fn schema_to_ts(schema: &Value, indent: usize) -> String {
    Gen { root: schema, refs: Vec::new() }.ts(schema, 0, indent)
}

pub fn signature(display: &str, description: &str, input: &Value, output: Option<&Value>) -> String {
    let input_ts = schema_to_ts(input, 0);
    let arg = if input_has_required(input) { "input" } else { "input?" };
    let out = output.map(|o| schema_to_ts(o, 0)).unwrap_or_else(|| "unknown".into());
    format!("{}{display}({arg}: {input_ts}): Promise<{out}>;", doc_block(description, ""))
}

pub fn input_has_required(input: &Value) -> bool {
    input.get("required").and_then(Value::as_array).is_some_and(|r| !r.is_empty())
}
