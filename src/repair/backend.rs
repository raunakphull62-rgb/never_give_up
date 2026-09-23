//! Model backends: `ModelBackend` trait + OpenAI-compatible curl impl + mock.
//!
//! Std-only HTTPS: shells out to the system `curl` binary (TLS included),
//! so no new Cargo deps. `MockBackend` backs all unit tests (no network).

/// Backend failure modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    MissingConfig(String),
    Http(String),
    Timeout,
    Parse(String),
    Api(String),
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingConfig(s) => write!(f, "missing config: {s}"),
            Self::Http(s) => write!(f, "http error: {s}"),
            Self::Timeout => write!(f, "request timed out"),
            Self::Parse(s) => write!(f, "parse error: {s}"),
            Self::Api(s) => write!(f, "api error: {s}"),
        }
    }
}

/// One model call: system + user prompt -> raw Klang source text.
/// v1 ships one OpenAI-compatible `/v1/chat/completions` impl (covers the
/// Kaggle/ngrok DeepSeek harness and most self-hosted/hosted options).
pub trait ModelBackend {
    fn complete(&self, system: &str, user: &str) -> Result<String, ModelError>;
    /// How many calls have been made (mocks assert on this; real returns 0).
    fn calls(&self) -> usize {
        0
    }
}

/// Escape a string for embedding in a JSON double-quoted value.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Build the OpenAI chat-completions request body (pure, unit-tested).
pub fn chat_request_body(model: &str, system: &str, user: &str) -> String {
    format!(
        "{{\"model\": \"{}\", \"temperature\": 0, \"messages\": [{{\"role\": \"system\", \"content\": \"{}\"}}, {{\"role\": \"user\", \"content\": \"{}\"}}]}}",
        json_escape(model),
        json_escape(system),
        json_escape(user)
    )
}

/// Normalize an endpoint: accept a bare base URL or a full path.
/// `http://host:8000` -> `http://host:8000/v1/chat/completions`;
/// anything already containing `chat/completions` passes through.
pub fn chat_url(endpoint: &str) -> String {
    let e = endpoint.trim().trim_end_matches('/');
    if e.contains("chat/completions") {
        return e.to_string();
    }
    if e.ends_with("/v1") {
        return format!("{e}/chat/completions");
    }
    format!("{e}/v1/chat/completions")
}

/// OpenAI-compatible backend over the system `curl` binary.
pub struct OpenAiCurlBackend {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
    pub timeout_secs: u64,
}

impl OpenAiCurlBackend {
    pub fn new(endpoint: &str, model: &str, api_key: &str, timeout_secs: u64) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            model: model.to_string(),
            api_key: api_key.to_string(),
            timeout_secs: timeout_secs.max(1),
        }
    }
    fn run_curl(&self, body: &str) -> Result<String, ModelError> {
        use std::process::Command;
        let url = chat_url(&self.endpoint);
        let mut cmd = Command::new("curl");
        cmd.arg("-sS")
            .arg("--max-time")
            .arg(self.timeout_secs.to_string())
            .arg("-X")
            .arg("POST")
            .arg("-H")
            .arg("Content-Type: application/json");
        if !self.api_key.is_empty() {
            cmd.arg("-H").arg(format!("Authorization: Bearer {}", self.api_key));
        }
        let out = cmd
            .arg("--data-binary")
            .arg("@-")
            .arg(&url)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| ModelError::Http(format!("cannot run curl: {e}")))?;
        use std::io::Write;
        let mut child = out;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(body.as_bytes()).map_err(|e| ModelError::Http(format!("curl stdin: {e}")))?;
        }
        let res = child.wait_with_output().map_err(|e| ModelError::Http(format!("curl wait: {e}")))?;
        if !res.status.success() {
            let err = String::from_utf8_lossy(&res.stderr).to_string();
            if err.contains("timed out") || err.contains("Timeout") || err.contains("28") && err.contains("time") {
                return Err(ModelError::Timeout);
            }
            return Err(ModelError::Http(format!("curl exit {}: {}", res.status, err.trim())));
        }
        String::from_utf8(res.stdout).map_err(|e| ModelError::Parse(format!("non-utf8 response: {e}")))
    }
}

impl ModelBackend for OpenAiCurlBackend {
    fn complete(&self, system: &str, user: &str) -> Result<String, ModelError> {
        let body = chat_request_body(&self.model, system, user);
        let raw = self.run_curl(&body)?;
        extract_content(&raw)
    }
}

/// Extract the assistant `content` string from a chat-completions response.
/// Hand-rolled (no serde): finds `"choices"`, then the first `"content"`
/// string value, honoring JSON escapes. Falls back to treating the whole
/// body as raw text when no choices envelope is present (some harnesses
/// return plain text).
pub fn extract_content(raw: &str) -> Result<String, ModelError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(ModelError::Parse("empty model response".into()));
    }
    if !t.contains("\"choices\"") {
        // Plain-text harness: strip optional code fences, return as-is.
        return Ok(strip_fences(t));
    }
    let key = "\"content\"";
    let mut search = 0usize;
    while let Some(rel) = t[search..].find(key) {
        let mut i = search + rel + key.len();
        while t.as_bytes().get(i).is_some_and(|b| b.is_ascii_whitespace() || *b == b':') {
            // advance past whitespace/colon (colon belongs to the kv sep)
            if t.as_bytes()[i] == b':' {
                i += 1;
                break;
            }
            i += 1;
        }
        while t.as_bytes().get(i).is_some_and(|b| b.is_ascii_whitespace()) {
            i += 1;
        }
        if t.as_bytes().get(i) == Some(&b'"') {
            match read_json_string(&t[i..]) {
                Some((s, _)) => return Ok(strip_fences(&s)),
                None => return Err(ModelError::Parse("unterminated content string".into())),
            }
        }
        search = i.max(search + 1);
        if search >= t.len() {
            break;
        }
    }
    Err(ModelError::Parse("no content field in model response".into()))
}

/// Read one JSON string literal (starting quote included). Returns
/// (decoded, bytes consumed including quotes).
fn read_json_string(s: &str) -> Option<(String, usize)> {
    let b = s.as_bytes();
    if b.first() != Some(&b'"') {
        return None;
    }
    let mut out = String::new();
    let mut i = 1usize;
    while i < b.len() {
        match b[i] {
            b'"' => return Some((out, i + 1)),
            b'\\' => {
                i += 1;
                if i >= b.len() {
                    return None;
                }
                match b[i] {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        if i + 4 >= b.len() {
                            return None;
                        }
                        let hex = &s[i + 1..i + 5];
                        let cp = u32::from_str_radix(hex, 16).ok()?;
                        out.push(char::from_u32(cp)?);
                        i += 4;
                    }
                    c => {
                        out.push('\\');
                        out.push(c as char);
                    }
                }
                i += 1;
            }
            _ => {
                // copy one UTF-8 char
                let ch = s[i..].chars().next()?;
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    None
}

/// Strip ``` fences (```klang ... ```) when the model wraps its answer.
pub fn strip_fences(s: &str) -> String {
    let t = s.trim();
    if !t.starts_with("```") {
        return t.to_string();
    }
    let mut lines: Vec<&str> = t.lines().collect();
    lines.remove(0);
    if lines.last().is_some_and(|l| l.trim().starts_with("```")) {
        lines.pop();
    }
    lines.join("\n").trim().to_string()
}

/// Deterministic mock backend for tests and `--dry-run` shape checks.
/// Returns queued responses in order; repeats the last when exhausted.
pub struct MockBackend {
    pub responses: std::sync::Mutex<Vec<String>>,
    pub calls: std::sync::Mutex<usize>,
    pub fail_with: Option<String>,
}

impl MockBackend {
    pub fn new(responses: Vec<String>) -> Self {
        Self { responses: std::sync::Mutex::new(responses), calls: std::sync::Mutex::new(0), fail_with: None }
    }
    pub fn failing(msg: &str) -> Self {
        Self { responses: std::sync::Mutex::new(vec![]), calls: std::sync::Mutex::new(0), fail_with: Some(msg.to_string()) }
    }
}

impl ModelBackend for MockBackend {
    fn complete(&self, _system: &str, _user: &str) -> Result<String, ModelError> {
        *self.calls.lock().unwrap() += 1;
        if let Some(m) = &self.fail_with {
            return Err(ModelError::Http(m.clone()));
        }
        let mut q = self.responses.lock().unwrap();
        if q.is_empty() {
            return Err(ModelError::Parse("mock: no more queued responses".into()));
        }
        if q.len() > 1 {
            Ok(q.remove(0))
        } else {
            Ok(q[0].clone())
        }
    }
    fn calls(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_normalizes() {
        assert_eq!(chat_url("http://h:8000"), "http://h:8000/v1/chat/completions");
        assert_eq!(chat_url("http://h:8000/v1"), "http://h:8000/v1/chat/completions");
        assert_eq!(chat_url("http://h/v1/chat/completions"), "http://h/v1/chat/completions");
    }
    #[test]
    fn body_is_json_shaped() {
        let b = chat_request_body("m", "sys\"q", "u\nx");
        assert!(b.contains("\"temperature\": 0"), "{b}");
        assert!(b.contains("sys\\\"q"), "{b}");
        assert!(b.contains("u\\nx"), "{b}");
    }
    #[test]
    fn extracts_choices_content() {
        let raw = "{\"choices\": [{\"message\": {\"role\": \"assistant\", \"content\": \"fn main() -> i32 { return 1 }\"}}]}";
        assert_eq!(extract_content(raw).unwrap(), "fn main() -> i32 { return 1 }");
        let esc = "{\"choices\": [{\"message\": {\"content\": \"a\\nb\"}}]}";
        assert_eq!(extract_content(esc).unwrap(), "a\nb");
    }
    #[test]
    fn plain_text_and_fences() {
        assert_eq!(extract_content("fn main() -> i32 { return 1 }").unwrap(), "fn main() -> i32 { return 1 }");
        assert_eq!(strip_fences("```klang\nfn main() -> i32 { return 1 }\n```"), "fn main() -> i32 { return 1 }");
    }
}
