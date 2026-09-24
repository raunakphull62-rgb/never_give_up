//! Structured diagnostics: objects, not strings.
//! Consumed identically by the terminal renderer, IDE, and AI repair loop.

use std::fmt;

/// Byte span inside one source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub file: String,
    pub start: usize,
    pub end: usize,
}

/// One machine-applicable repair suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fix {
    pub label: String,
}

/// A single compiler-raised error as a structured object.
/// `related` carries grouped child failures (`E-TASK-GROUP`); empty
/// everywhere else, so all existing construction stays valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: String,
    pub severity: String,
    pub message: String,
    pub primary_span: Span,
    pub cause: String,
    pub fixes: Vec<Fix>,
    pub rule: String,
    pub related: Vec<Diagnostic>,
}

impl Diagnostic {
    pub fn error(
        code: &str,
        message: &str,
        file: &str,
        start: usize,
        end: usize,
        cause: &str,
        fix_labels: &[&str],
        rule: &str,
    ) -> Self {
        Self {
            code: code.to_string(),
            severity: "error".to_string(),
            message: message.to_string(),
            primary_span: Span {
                file: file.to_string(),
                start,
                end,
            },
            cause: cause.to_string(),
            fixes: fix_labels
                .iter()
                .map(|l| Fix {
                    label: l.to_string(),
                })
                .collect(),
            rule: rule.to_string(),
            related: Vec::new(),
        }
    }

    /// Task-group failure aggregate: every child failure, none dropped
    /// (Python `ExceptionGroup` semantics). Single failures stay unwrapped.
    pub fn task_group(failures: Vec<Diagnostic>) -> Self {
        Self {
            code: "E-TASK-GROUP".to_string(),
            severity: "error".to_string(),
            message: format!("task group failed with {} errors", failures.len()),
            primary_span: Span {
                file: "runtime".to_string(),
                start: 0,
                end: 0,
            },
            cause: "one or more child tasks failed; siblings were cancelled and joined".to_string(),
            fixes: vec![Fix {
                label: "fix each related failure".to_string(),
            }],
            rule: "structured-concurrency/failure".to_string(),
            related: failures,
        }
    }

    /// Cooperative-cancellation notice: excluded from group aggregates.
    pub fn cancelled(handle: &str) -> Self {        Self::error(
            "E-CANCELLED",
            &format!("task `{handle}` was cancelled after a sibling failed"),
            "runtime",
            0,
            0,
            "cooperative cancellation at an await or loop checkpoint",
            &[],
            "structured-concurrency/cancel",
        )
    }

    /// True for cooperative-cancellation notices (excluded from groups).
    pub fn is_cancelled(&self) -> bool {
        self.code == "E-CANCELLED"
    }

    /// Scope-violation diagnostic: a spawned handle escapes its group.
    pub fn task_leak(file: &str, start: usize, end: usize, handle: &str) -> Self {
        Self::error(
            "E-TASK-CANCEL",
            &format!("child task `{handle}` may outlive its task group"),
            file,
            start,
            end,
            "spawned task handle is not awaited or detached explicitly",
            &["await task before leaving scope", "mark task detached"],
            "structured-concurrency/lifetime",
        )
    }

    /// `spawn` with no enclosing group.
    pub fn spawn_outside_group(file: &str, start: usize, end: usize) -> Self {
        Self::error(
            "E-SPAWN-OUTSIDE-GROUP",
            "spawn outside of any task_group",
            file,
            start,
            end,
            "spawn creates a child task tied to the enclosing task_group",
            &["wrap the spawn in a task_group block"],
            "structured-concurrency/scope",
        )
    }

    /// `spawn` where no handle can be bound (nested in an expression or a
    /// bare statement): the thread could never be awaited or joined.
    pub fn spawn_position(file: &str) -> Self {
        Self::error(
            "E-SPAWN-POSITION",
            "spawn must be directly bound by `let h = spawn f()`",
            file,
            0,
            0,
            "only a direct let-binding gives the handle a name to await",
            &["bind it with `let` first", "remove the nested spawn"],
            "structured-concurrency/scope",
        )
    }

    /// Caller lacks an effect the callee requires.
    pub fn effect_mismatch(
        file: &str,
        start: usize,
        end: usize,
        caller: &str,
        callee: &str,
        effect: &str,
    ) -> Self {
        Self::error(
            "E-EFFECT-MISMATCH",
            &format!("`{caller}` calls throwing `{callee}` without `{effect}`"),
            file,
            start,
            end,
            &format!("callee requires visible `{effect}` effect in caller signature"),
            &["declare the effect in the caller signature"],
            "effects/visibility",
        )
    }

    /// Cross-module reference to a private declaration.
    pub fn private_access(file: &str, start: usize, end: usize, path: &str) -> Self {
        Self::error(
            "E-PRIVATE",
            &format!("`{path}` is private and cannot be referenced from another module"),
            file,
            start,
            end,
            "only `pub` declarations are visible across module boundaries",
            &["mark the declaration `pub`", "use it from inside its own module"],
            "modules/visibility",
        )
    }

    /// Generic parse error.
    pub fn parse_error(file: &str, start: usize, end: usize, message: &str) -> Self {
        Self::error(
            "E-PARSE",
            message,
            file,
            start,
            end,
            "input does not match the v0.1 grammar",
            &[],
            "syntax/grammar",
        )
    }

    fn esc(s: &str) -> String {
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

    /// Render as a JSON object per the architecture schema.
    pub fn to_json(&self) -> String {
        let fixes: Vec<String> = self
            .fixes
            .iter()
            .map(|f| format!("{{\"label\": \"{}\"}}", Self::esc(&f.label)))
            .collect();
        let related: Vec<String> = self.related.iter().map(|d| d.to_json()).collect();
        format!(
            "{{\n  \"code\": \"{}\",\n  \"severity\": \"{}\",\n  \"message\": \"{}\",\n  \"primary_span\": {{\"file\": \"{}\", \"start\": {}, \"end\": {}}},\n  \"cause\": \"{}\",\n  \"fixes\": [{}],\n  \"rule\": \"{}\",\n  \"related\": [{}]\n}}",
            Self::esc(&self.code),
            Self::esc(&self.severity),
            Self::esc(&self.message),
            Self::esc(&self.primary_span.file),
            self.primary_span.start,
            self.primary_span.end,
            Self::esc(&self.cause),
            fixes.join(", "),
            Self::esc(&self.rule),
            related.join(", "),
        )
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}] {}:{}-{} {} (cause: {}; rule: {})",
            self.code,
            self.primary_span.file,
            self.primary_span.start,
            self.primary_span.end,
            self.message,
            self.cause,
            self.rule
        )
    }
}

/// Render a user- or file-derived string safe for raw terminal output.
///
/// Diagnostic JSON already escapes control characters, but the CLI also
/// prints some file-derived values (import paths, OS errors) as plain
/// text. A malicious `.klang` file can smuggle raw ESC bytes into those
/// paths (`import "<ESC>[2J"`), turning a routine error into terminal
/// escape injection on the victim's screen. This renders control
/// characters as visible `\u{...}` escapes while leaving normal text
/// (including non-ASCII) untouched.
pub fn sanitize_for_terminal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_control() {
            out.push_str(&format!("\\u{{{:x}}}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}
