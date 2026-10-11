//! Shared hand-rolled JSON value model (parse + serialize, no dependencies).
//!
//! Shared by the LSP server (`src/lsp.rs`), the registry client
//! (`src/registry.rs`), and the CLI `--json` output. No new dependencies.

/// Maximum JSON nesting accepted in requests (adversarial-depth guard
/// for the recursive parser below).
const MAX_JSON_DEPTH: usize = 64;

// ---------------------------------------------------------------------
// Minimal JSON value model (parse + serialize, no dependencies).
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&Vec<Json>> {
        match self {
            Json::Arr(v) => Some(v),
            _ => None,
        }
    }
    fn esc_into(out: &mut String, s: &str) {
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                '\u{08}' => out.push_str("\\b"),
                '\u{0c}' => out.push_str("\\f"),
                c if (c as u32) < 0x20 => {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => out.push(c),
            }
        }
    }
    pub fn render(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }
    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Int(n) => out.push_str(&n.to_string()),
            Json::Float(f) => {
                if f.is_finite() {
                    out.push_str(&format!("{f:?}"));
                } else {
                    out.push_str("null");
                }
            }
            Json::Str(s) => {
                out.push('"');
                Self::esc_into(out, s);
                out.push('"');
            }
            Json::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Json::Obj(pairs) => {
                out.push('{');
                for (i, (k, v)) in pairs.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push('"');
                    Self::esc_into(out, k);
                    out.push_str("\":");
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

fn parse_string(s: &str, b: &[u8], i: usize) -> Result<(String, usize), String> {
    // b[i] == b'"' on entry.
    let mut out = String::new();
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'"' => return Ok((out, j + 1)),
            b'\\' => {
                j += 1;
                if j >= b.len() {
                    return Err("unterminated escape".to_string());
                }
                match b[j] {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{08}'),
                    b'f' => out.push('\u{0c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        if j + 4 >= b.len() {
                            return Err("truncated \\u escape".to_string());
                        }
                        let hex = &s[j + 1..j + 5];
                        let cp = u32::from_str_radix(hex, 16)
                            .map_err(|_| "bad \\u escape".to_string())?;
                        out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                        j += 4;
                    }
                    _ => return Err("bad escape".to_string()),
                }
                j += 1;
            }
            _ => {
                let ch = s[j..].chars().next().ok_or("bad utf-8")?;
                out.push(ch);
                j += ch.len_utf8();
            }
        }
    }
    Err("unterminated string".to_string())
}

fn parse_number(s: &str, b: &[u8], i: usize) -> Result<(Json, usize), String> {
    let mut j = i;
    if b.get(j) == Some(&b'-') {
        j += 1;
    }
    let int_start = j;
    while j < b.len() && b[j].is_ascii_digit() {
        j += 1;
    }
    let mut is_float = false;
    if b.get(j) == Some(&b'.') {
        is_float = true;
        j += 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
    }
    if b.get(j) == Some(&b'e') || b.get(j) == Some(&b'E') {
        is_float = true;
        j += 1;
        if b.get(j) == Some(&b'+') || b.get(j) == Some(&b'-') {
            j += 1;
        }
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
    }
    if j == int_start {
        return Err("bad number".to_string());
    }
    let text = &s[i..j];
    if !is_float {
        if let Ok(n) = text.parse::<i64>() {
            return Ok((Json::Int(n), j));
        }
    }
    text.parse::<f64>()
        .map(|f| (Json::Float(f), j))
        .map_err(|_| "bad number".to_string())
}

fn parse_value(s: &str, b: &[u8], i: usize, depth: usize) -> Result<(Json, usize), String> {
    if depth > MAX_JSON_DEPTH {
        return Err("json too deeply nested".to_string());
    }
    let i = skip_ws(b, i);
    if i >= b.len() {
        return Err("unexpected end of json".to_string());
    }
    match b[i] {
        b'"' => parse_string(s, b, i).map(|(v, j)| (Json::Str(v), j)),
        b'{' => {
            let mut pairs = Vec::new();
            let mut j = skip_ws(b, i + 1);
            if b.get(j) == Some(&b'}') {
                return Ok((Json::Obj(pairs), j + 1));
            }
            loop {
                j = skip_ws(b, j);
                if b.get(j) != Some(&b'"') {
                    return Err("expected string key".to_string());
                }
                let (k, j2) = parse_string(s, b, j)?;
                j = skip_ws(b, j2);
                if b.get(j) != Some(&b':') {
                    return Err("expected ':'".to_string());
                }
                let (v, j3) = parse_value(s, b, j + 1, depth + 1)?;
                pairs.push((k, v));
                j = skip_ws(b, j3);
                match b.get(j) {
                    Some(&b',') => j += 1,
                    Some(&b'}') => return Ok((Json::Obj(pairs), j + 1)),
                    _ => return Err("expected ',' or '}'".to_string()),
                }
            }
        }
        b'[' => {
            let mut items = Vec::new();
            let mut j = skip_ws(b, i + 1);
            if b.get(j) == Some(&b']') {
                return Ok((Json::Arr(items), j + 1));
            }
            loop {
                let (v, j2) = parse_value(s, b, j, depth + 1)?;
                items.push(v);
                j = skip_ws(b, j2);
                match b.get(j) {
                    Some(&b',') => j += 1,
                    Some(&b']') => return Ok((Json::Arr(items), j + 1)),
                    _ => return Err("expected ',' or ']'".to_string()),
                }
            }
        }
        b't' if s[i..].starts_with("true") => Ok((Json::Bool(true), i + 4)),
        b'f' if s[i..].starts_with("false") => Ok((Json::Bool(false), i + 5)),
        b'n' if s[i..].starts_with("null") => Ok((Json::Null, i + 4)),
        c if c == b'-' || c.is_ascii_digit() => parse_number(s, b, i),
        _ => Err("unexpected json value".to_string()),
    }
}

/// Parse one complete JSON value; trailing garbage is an error.
pub fn parse_json(s: &str) -> Result<Json, String> {
    let b = s.as_bytes();
    let (v, j) = parse_value(s, b, 0, 0)?;
    if skip_ws(b, j) != b.len() {
        return Err("trailing characters after json".to_string());
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip() {
        let v = parse_json("{\"a\": [1, -2.5, true, null, \"x\\ny\"]}").expect("parses");
        assert_eq!(v.get("a").and_then(Json::as_arr).map(Vec::len), Some(5));
        let again = parse_json(&v.render()).expect("reparses");
        assert_eq!(v, again);
    }

    #[test]
    fn json_rejects_garbage_and_depth() {
        assert!(parse_json("{oops").is_err());
        assert!(parse_json("{\"a\": 1} trailing").is_err());
        assert!(parse_json("").is_err());
        let mut deep = String::new();
        for _ in 0..200 {
            deep.push('[');
        }
        assert!(parse_json(&deep).is_err());
    }
}
