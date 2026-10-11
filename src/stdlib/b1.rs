//! B1 stdlib gap fill (Lane B, stage B1).
//!
//! Native complements to the pure-Klang `stdlib-packages/` libraries, chosen
//! so no name collides with an existing package function (a same-named
//! native would trip `W-SHADOW`/`E-DUPLICATE` and break the package's parity
//! self-test — see `docs/progress/B1-audit.md`):
//!
//! - regex complements over the `regex` crate: `regex_find_all`,
//!   `regex_replace` (`regex_is_match`/`regex_find` already exist).
//! - `uuid_v4` (no package covers UUIDs).
//! - `path_absolute` (packages cover join/dirname/basename/extension/
//!   normalize as pure `/` string code; only the CWD-dependent absolute
//!   form needs the OS).
//! - `file_mtime` (packages/`fs` cover every other file-util row).
//! - `sprintf` (the existing `format` joins with spaces; this is the
//!   `{}`-placeholder form).
//! - `random_seed` / `random_seeded_int` / `random_seeded_float`
//!   (deterministic global RNG; the `random` package stays the
//!   explicit-state story, `random_int`/`random_float` stay OS-seeded).
//! - char-based `"s".pad_start/pad_end/repeat/lines` method helpers
//!   (the `text`/`string` packages cover these in pure Klang with
//!   different edge rules; see each function's docs).
//!
//! Flat names are deliberate, matching the existing `read_file` /
//! `regex_is_match` convention (`::` spellings do not resolve through
//! program modules). All user-input failures are loud diagnostics with
//! `E-<AREA>-<REASON>` codes, never panics. Output/bomb guards:
//! `regex_find_all` (10k matches), `regex_replace` (1M output bytes),
//! `repeat`/pads (1M chars).

use std::sync::Mutex;

use crate::diagnostics::Diagnostic;

// ---------------------------------------------------------------------------
// Shared limits.
// ---------------------------------------------------------------------------

/// Max matches `regex_find_all` collects before `E-REGEX-TOO-MANY`.
pub const MAX_FIND_ALL: usize = 10_000;
/// Max output bytes `regex_replace` builds before `E-REGEX-TOO-LARGE`.
pub const MAX_REPLACE_BYTES: usize = 1_000_000;
/// Max chars `repeat`/pads build before `E-STR-TOO-LARGE`.
pub const MAX_STR_CHARS: usize = 1_000_000;
/// Template text shown verbatim in `E-FMT-*` messages past this many chars.
pub const MAX_SHOWN_FMT_CHARS: usize = 200;

// ---------------------------------------------------------------------------
// Error constructors (every message names the value involved).
// ---------------------------------------------------------------------------

/// `E-REGEX-TOO-MANY`: `pattern` matched more than [`MAX_FIND_ALL`] times.
fn regex_too_many(op: &str, pattern: &str, limit: usize) -> Diagnostic {
    Diagnostic::error(
        "E-REGEX-TOO-MANY",
        &format!("{op}({pattern}) failed: more than {limit} matches"),
        "runtime",
        0,
        0,
        "the pattern matches too many times; collect in slices instead",
        &["narrow the pattern", "split the input and search each part"],
        "regex/limits",
    )
}

/// `E-REGEX-TOO-LARGE`: replacement output passed [`MAX_REPLACE_BYTES`].
fn regex_too_large(op: &str, pattern: &str, limit: usize) -> Diagnostic {
    Diagnostic::error(
        "E-REGEX-TOO-LARGE",
        &format!("{op}({pattern}) failed: output larger than {limit} bytes"),
        "runtime",
        0,
        0,
        "the replacement expands the input beyond the output limit",
        &["replace in slices", "use a narrower pattern"],
        "regex/limits",
    )
}

/// Render a template for diagnostics, redacting past
/// [`MAX_SHOWN_FMT_CHARS`] characters so a multi-megabyte template never
/// lands verbatim in a diagnostic.
fn redact_fmt(fmt: &str) -> String {
    if fmt.chars().count() <= MAX_SHOWN_FMT_CHARS {
        return format!("\"{fmt}\"");
    }
    let head: String = fmt.chars().take(MAX_SHOWN_FMT_CHARS).collect();
    let rest = fmt.chars().count() - MAX_SHOWN_FMT_CHARS;
    format!("\"{head}...\" [{rest} chars redacted]")
}

/// `E-FMT-INVALID`: `fmt` has a lone `{`/`}` outside `{{`/`}}`/`{}`.
fn fmt_invalid(fmt: &str, detail: &str) -> Diagnostic {
    Diagnostic::error(
        "E-FMT-INVALID",
        &format!("sprintf({}) failed: {detail}", redact_fmt(fmt)),
        "runtime",
        0,
        0,
        "sprintf templates use {} for values and {{{{}}}} for literal braces",
        &["use {} for each value", "write {{ for a literal {"],
        "fmt/template",
    )
}

/// `E-FMT-ARG-MISMATCH`: more `{}` placeholders than `args` entries.
fn fmt_arg_mismatch(fmt: &str, want: usize, got: usize) -> Diagnostic {
    Diagnostic::error(
        "E-FMT-ARG-MISMATCH",
        &format!(
            "sprintf({}) failed: {want} placeholders but {got} args",
            redact_fmt(fmt)
        ),
        "runtime",
        0,
        0,
        "every {} in the template needs one entry in the args array",
        &["add the missing args", "remove the extra {}"],
        "fmt/arity",
    )
}

/// `E-STR-INVALID`: bad pad/repeat shape (empty pad, negative width/count).
fn str_invalid(op: &str, detail: &str) -> Diagnostic {
    Diagnostic::error(
        "E-STR-INVALID",
        &format!("{op} failed: {detail}"),
        "runtime",
        0,
        0,
        "pad needs a non-empty pad string and width >= 0; repeat needs count >= 0",
        &["check the width/count and pad first"],
        "str/invalid",
    )
}

/// `E-STR-TOO-LARGE`: pad/repeat output past [`MAX_STR_CHARS`] chars.
fn str_too_large(op: &str, want: usize, limit: usize) -> Diagnostic {
    Diagnostic::error(
        "E-STR-TOO-LARGE",
        &format!("{op} failed: output would be {want} chars (limit {limit})"),
        "runtime",
        0,
        0,
        "padded/repeated output is capped so a typo cannot OOM the host",
        &["use a smaller width/count", "build the string in parts"],
        "str/limits",
    )
}

/// `E-RANDOM-BOUNDS`: `lo > hi` for the seeded int draw (mirrors the
/// OS-seeded `random_int` loudness, with a stable code of its own).
fn random_bounds(op: &str, lo: i64, hi: i64) -> Diagnostic {
    Diagnostic::error(
        "E-RANDOM-BOUNDS",
        &format!("{op}({lo}, {hi}) failed: lo > hi"),
        "runtime",
        0,
        0,
        "the lower bound must not exceed the upper bound",
        &["swap the bounds", "check the bound order"],
        "random/bounds",
    )
}

// ---------------------------------------------------------------------------
// Regex complements.
// ---------------------------------------------------------------------------

/// All non-overlapping matches of `pattern` in `text`, in order.
/// Empty match list is a normal result (no match, not an error).
/// Bad patterns reuse `E-REGEX-INVALID-PATTERN` via the shared mapper;
/// more than [`MAX_FIND_ALL`] matches is `E-REGEX-TOO-MANY`.
pub fn find_all(pattern: &str, text: &str) -> Result<Vec<String>, Diagnostic> {
    let re = regex::Regex::new(pattern)
        .map_err(|e| crate::stdlib::regex::map_regex_error("regex_find_all", pattern, &e))?;
    let mut out = Vec::new();
    for m in re.find_iter(text) {
        if out.len() >= MAX_FIND_ALL {
            return Err(regex_too_many("regex_find_all", pattern, MAX_FIND_ALL));
        }
        out.push(m.as_str().to_string());
    }
    Ok(out)
}

/// Replace every non-overlapping match of `pattern` in `text` with `repl`.
/// `repl` uses the `regex`-crate `$1`/`$name`/`$$` expansion. Bad patterns
/// reuse `E-REGEX-INVALID-PATTERN`; output past [`MAX_REPLACE_BYTES`]
/// bytes is `E-REGEX-TOO-LARGE` (checked after the crate call; inputs far
/// past the limit should be sliced first — documented, not streamed).
pub fn replace(pattern: &str, text: &str, repl: &str) -> Result<String, Diagnostic> {
    let re = regex::Regex::new(pattern)
        .map_err(|e| crate::stdlib::regex::map_regex_error("regex_replace", pattern, &e))?;
    let done = re.replace_all(text, repl).into_owned();
    if done.len() > MAX_REPLACE_BYTES {
        return Err(regex_too_large("regex_replace", pattern, MAX_REPLACE_BYTES));
    }
    Ok(done)
}

// ---------------------------------------------------------------------------
// UUID v4.
// ---------------------------------------------------------------------------

/// OS entropy, same two-tier source as the OS-seeded `random_int` draws:
/// 16 bytes from `/dev/urandom` on Unix, else SplitMix64 over
/// (wall-clock nanos ^ pid ^ per-process counter). Never fails and never
/// panics; the fallback is documented non-cryptographic either way.
fn entropy16() -> [u8; 16] {
    let mut buf = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        if f.read_exact(&mut buf).is_ok() {
            return buf;
        }
    }
    static CTR: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(0x243F6A8885A308D3);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x12345678);
    let mut seed = nanos
        ^ (std::process::id() as u64).wrapping_mul(0x9E3779B97F4A7C15)
        ^ CTR
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .wrapping_mul(0xBF58476D1CE4E5B9);
    for chunk in buf.chunks_mut(8) {
        seed = seed.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        chunk.copy_from_slice(&z.to_le_bytes()[..chunk.len()]);
    }
    buf
}

/// Random UUID v4 (`xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`, `y` in
/// `8/9/a/b`). Infallible: entropy always yields 16 bytes via the
/// fallback above. NOT FOR SECURITY--critical tokens on hosts without
/// `/dev/urandom` (same caveat as `random_int`).
pub fn uuid_v4() -> String {
    let mut b = entropy16();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9],
        b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

// ---------------------------------------------------------------------------
// Path absolute (lexical, no FS touch).
// ---------------------------------------------------------------------------

/// Make `p` absolute by joining the process CWD and normalizing lexically
/// (`.` dropped, `..` pops, clamped at the root). Never touches the FS,
/// so it works for not-yet-existing paths (what C1/C4 pass before
/// creating files); never fails, never panics. Separators are
/// platform-native (`Path`/`PathBuf`); the pure-`/` package helpers stay
/// for portable string-only work.
pub fn absolute(p: &str) -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let joined = if p.is_empty() {
        cwd
    } else {
        let rel = std::path::Path::new(p);
        if rel.is_absolute() {
            rel.to_path_buf()
        } else {
            cwd.join(rel)
        }
    };
    // Lexical normalize via components (portable: Prefix/RootDir kept).
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for comp in joined.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    // Clamped at root/empty: `..` past the top is dropped
                    // (same rule as the `path_normalize` package).
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        return ".".to_string();
    }
    out.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------
// File mtime.
// ---------------------------------------------------------------------------

/// Seconds since the Unix epoch of `path`'s modification time, as an
/// `f64` on the same clock as `time_now()`. Missing files map to
/// `E-IO-NOT-FOUND`, permission failures to `E-IO-PERMISSION`, other OS
/// failures to `E-IO-FAILED` via the shared [`crate::stdlib::file`]
/// mapper. Pre-epoch times (clock oddity) read as `0.0`, like `now()`.
pub fn mtime(path: &str) -> Result<f64, Diagnostic> {
    let meta =
        std::fs::metadata(path).map_err(|e| crate::stdlib::file::map_io_error("file_mtime", path, &e))?;
    let t = meta
        .modified()
        .map_err(|e| crate::stdlib::file::map_io_error("file_mtime", path, &e))?;
    Ok(t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0))
}

// ---------------------------------------------------------------------------
// sprintf.
// ---------------------------------------------------------------------------

/// `sprintf(fmt, args)`: `{}` takes the next arg (rendered exactly like
/// `str()`), `{{`/`}}` are literal braces. Extra args are ignored
/// (documented); fewer args than placeholders is `E-FMT-ARG-MISMATCH`;
/// a lone `{`/`}` is `E-FMT-INVALID`. Zero placeholders returns `fmt`
/// with escapes resolved. No output cap (same unboundedness as `format`
/// and string `+`).
pub fn sprintf(fmt: &str, args: &[crate::runtime::Value]) -> Result<String, Diagnostic> {
    let mut out = String::new();
    let mut argi: usize = 0;
    let mut it = fmt.chars().peekable();
    let mut placeholders: usize = 0;
    while let Some(c) = it.next() {
        if c == '{' {
            match it.peek() {
                Some('{') => {
                    it.next();
                    out.push('{');
                }
                Some('}') => {
                    it.next();
                    placeholders += 1;
                    let v = args.get(argi).ok_or_else(|| {
                        fmt_arg_mismatch(fmt, placeholders, args.len())
                    })?;
                    argi += 1;
                    out.push_str(&v.render());
                }
                _ => {
                    return Err(fmt_invalid(
                        fmt,
                        "lone `{` (use {} for a value or {{ for a literal brace)",
                    ));
                }
            }
        } else if c == '}' {
            match it.peek() {
                Some('}') => {
                    it.next();
                    out.push('}');
                }
                _ => {
                    return Err(fmt_invalid(
                        fmt,
                        "lone `}` (use }} for a literal brace)",
                    ));
                }
            }
        } else {
            out.push(c);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Seeded (deterministic) RNG.
// ---------------------------------------------------------------------------

/// Global deterministic state for `random_seed` et al. (SplitMix64).
/// Poisoning is impossible to observe: locks recover via `into_inner`.
static SEEDED_STATE: Mutex<u64> = Mutex::new(0);

fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

fn lock_seeded() -> std::sync::MutexGuard<'static, u64> {
    SEEDED_STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Seed the deterministic RNG (returns 1). Same seed replays the same
/// `random_seeded_int`/`random_seeded_float` sequence. Infallible.
pub fn seed(seed: i64) -> i64 {
    *lock_seeded() = seed as u64;
    1
}

fn next_seeded() -> u64 {
    let mut g = lock_seeded();
    *g = splitmix64(*g);
    *g
}

/// Deterministic uniform `i64` in `[lo, hi]` (inclusive) via rejection
/// sampling (no modulo bias; full-span maps bijectively). `lo > hi` is
/// `E-RANDOM-BOUNDS`. NOT FOR KEYS OR TOKENS (deterministic by design).
pub fn seeded_int(lo: i64, hi: i64) -> Result<i64, Diagnostic> {
    if lo > hi {
        return Err(random_bounds("random_seeded_int", lo, hi));
    }
    let span = (hi as i128 - lo as i128 + 1) as u128;
    if span > u64::MAX as u128 {
        return Ok(lo.wrapping_add(next_seeded() as i64));
    }
    let bound = span as u64;
    let threshold = u64::MAX - u64::MAX % bound;
    for _ in 0..1024 {
        let r = next_seeded();
        if r < threshold {
            return Ok(lo.wrapping_add((r % bound) as i64));
        }
    }
    Ok(lo)
}

/// Deterministic uniform `f64` in `[0.0, 1.0)` (53 bits over 2^53).
/// Infallible. NOT FOR KEYS OR TOKENS.
pub fn seeded_float() -> f64 {
    (next_seeded() >> 11) as f64 / 9007199254740992.0
}

// ---------------------------------------------------------------------------
// Char-based string method helpers.
// ---------------------------------------------------------------------------

/// Left-pad `s` to `width` CHARS with `pad` (cycled, truncated to the exact
/// width). `width <= char_len` returns `s` unchanged. Empty `pad` or
/// negative `width` is `E-STR-INVALID`; `width > 1M` is `E-STR-TOO-LARGE`.
/// Deviation from the `text_pad_left` package (which repeats the whole pad
/// and overshoots multi-char pads): this form is width-exact.
pub fn pad_start(s: &str, width: i64, pad: &str) -> Result<String, Diagnostic> {
    if width < 0 {
        return Err(str_invalid(
            "pad_start",
            &format!("width {width} is negative"),
        ));
    }
    if pad.is_empty() {
        return Err(str_invalid("pad_start", "pad string is empty"));
    }
    let have = s.chars().count() as i64;
    if width <= have {
        return Ok(s.to_string());
    }
    if width as usize > MAX_STR_CHARS {
        return Err(str_too_large("pad_start", width as usize, MAX_STR_CHARS));
    }
    let need = (width - have) as usize;
    let pcs: Vec<char> = pad.chars().collect();
    let mut pre = String::new();
    for i in 0..need {
        pre.push(pcs[i % pcs.len()]);
    }
    pre.push_str(s);
    Ok(pre)
}

/// Right-pad mirror of [`pad_start`] (same codes and limits).
pub fn pad_end(s: &str, width: i64, pad: &str) -> Result<String, Diagnostic> {
    if width < 0 {
        return Err(str_invalid(
            "pad_end",
            &format!("width {width} is negative"),
        ));
    }
    if pad.is_empty() {
        return Err(str_invalid("pad_end", "pad string is empty"));
    }
    let have = s.chars().count() as i64;
    if width <= have {
        return Ok(s.to_string());
    }
    if width as usize > MAX_STR_CHARS {
        return Err(str_too_large("pad_end", width as usize, MAX_STR_CHARS));
    }
    let need = (width - have) as usize;
    let pcs: Vec<char> = pad.chars().collect();
    let mut out = s.to_string();
    for i in 0..need {
        out.push(pcs[i % pcs.len()]);
    }
    Ok(out)
}

/// `s` repeated `n` times. `n == 0` (or empty `s`) is `""`; negative `n`
/// is `E-STR-INVALID` (the packages return `""` silently — documented
/// deviation); output past [`MAX_STR_CHARS`] chars is `E-STR-TOO-LARGE`.
pub fn repeat_str(s: &str, n: i64) -> Result<String, Diagnostic> {
    if n < 0 {
        return Err(str_invalid(
            "repeat",
            &format!("count {n} is negative"),
        ));
    }
    if n == 0 || s.is_empty() {
        return Ok(String::new());
    }
    let total = (s.chars().count() as i128) * (n as i128);
    if total > MAX_STR_CHARS as i128 {
        return Err(str_too_large("repeat", total as usize, MAX_STR_CHARS));
    }
    Ok(s.repeat(n as usize))
}

/// Split on `\n`, stripping one trailing `\r` per line (CRLF-safe).
/// Infallible.
pub fn split_lines(s: &str) -> Vec<String> {
    s.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect()
}
