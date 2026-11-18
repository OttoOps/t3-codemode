use serde_json::{Map, Value};

pub const MAX_CHARS: usize = 24_000;
pub const MARKER: &str = "--- TRUNCATED ---";
const MIN_SHARE: usize = 48;

fn size(v: &Value) -> usize {
    serde_json::to_string(v).map(|s| s.len()).unwrap_or(0)
}

fn waterfill(sizes: &[usize], total: usize) -> usize {
    let mut sorted = sizes.to_vec();
    sorted.sort_unstable();
    let mut left = total;
    for (i, s) in sorted.iter().enumerate() {
        let share = left / (sorted.len() - i);
        if *s > share {
            return share;
        }
        left -= s;
    }
    usize::MAX
}

fn cut_string(s: &str, budget: usize) -> Value {
    let full_chars = s.chars().count();
    let tail = format!(" {MARKER} ({full_chars} chars in full)");
    let (mut lo, mut hi) = (0usize, full_chars);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let cand: String = s.chars().take(mid).collect::<String>() + &tail;
        if size(&Value::String(cand)) <= budget {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let kept: String = s.chars().take(lo).collect();
    Value::String(kept + &tail)
}

fn shrink_array(items: &[Value], budget: usize) -> Value {
    let sizes: Vec<usize> = items.iter().map(size).collect();
    let mut keep = items.len();
    loop {
        let dropped = items.len() - keep;
        let marker = Value::String(format!("{MARKER} {dropped} more items"));
        let reserve = 2 + keep + if dropped > 0 { size(&marker) + 1 } else { 0 };
        let room = budget.saturating_sub(reserve);
        let cap = waterfill(&sizes[..keep], room);
        if keep <= 1 || cap >= MIN_SHARE {
            let mut out: Vec<Value> = items[..keep].iter().zip(&sizes).map(|(v, s)| shrink(v, (*s).min(cap))).collect();
            if dropped > 0 {
                out.push(marker);
            }
            return Value::Array(out);
        }
        keep = (keep / 2).max(1);
    }
}

fn shrink_object(map: &Map<String, Value>, budget: usize) -> Value {
    let entries: Vec<(&String, &Value)> = map.iter().collect();
    let sizes: Vec<usize> = entries.iter().map(|(_, v)| size(v)).collect();
    let key_cost: Vec<usize> = entries.iter().map(|(k, _)| size(&Value::String((*k).clone())) + 2).collect();
    let mut keep = entries.len();
    loop {
        let dropped = entries.len() - keep;
        let reserve = 2 + key_cost[..keep].iter().sum::<usize>() + if dropped > 0 { MARKER.len() + 24 } else { 0 };
        let room = budget.saturating_sub(reserve);
        let cap = waterfill(&sizes[..keep], room);
        if keep <= 1 || cap >= MIN_SHARE {
            let mut out = Map::new();
            for ((k, v), s) in entries[..keep].iter().zip(&sizes) {
                out.insert((*k).clone(), shrink(v, (*s).min(cap)));
            }
            if dropped > 0 {
                out.insert(MARKER.into(), Value::String(format!("{dropped} more keys")));
            }
            return Value::Object(out);
        }
        keep = (keep / 2).max(1);
    }
}

pub fn shrink(v: &Value, budget: usize) -> Value {
    if size(v) <= budget {
        return v.clone();
    }
    let marker = Value::String(MARKER.into());
    if budget < size(&marker) + 8 {
        return marker;
    }
    let mut target = budget;
    for _ in 0..6 {
        let out = match v {
            Value::String(s) => cut_string(s, target),
            Value::Array(items) => shrink_array(items, target),
            Value::Object(map) => shrink_object(map, target),
            _ => marker.clone(),
        };
        let got = size(&out);
        if got <= budget {
            return out;
        }
        target = target.saturating_sub(got - budget + 16);
    }
    marker
}

pub fn render(result: Value, logs: Vec<String>) -> String {
    let mut obj = Map::new();
    obj.insert("result".into(), result);
    if !logs.is_empty() {
        obj.insert("logs".into(), Value::Array(logs.into_iter().map(Value::String).collect()));
    }
    let full = Value::Object(obj);
    let text = full.to_string();
    if text.len() <= MAX_CHARS {
        return text;
    }
    let note = format!(
        "{MARKER} The output was {} chars, over the {MAX_CHARS} cap; the largest values were cut. Narrow the request to reduce response size.",
        text.len()
    );
    let budget = MAX_CHARS - note.len() - 20;
    let mut out = match shrink(&full, budget) {
        Value::Object(m) => m,
        other => {
            let mut m = Map::new();
            m.insert("result".into(), other);
            m
        }
    };
    out.insert("truncated".into(), Value::String(note));
    Value::Object(out).to_string()
}
