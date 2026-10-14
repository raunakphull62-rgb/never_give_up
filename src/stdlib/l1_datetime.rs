//! L1 datetime: UTC civil math over epoch-milliseconds (Wave 2 Stage L1).
//!
//! Canonical instant type is an `Int` of milliseconds since the Unix epoch
//! (like `now_ms`, which already flows as an `i64` value). All parts and
//! formatting are UTC unless an explicit fixed offset is given
//! (`dt_parts_offset`); named time zones are unsupported (documented).
//!
//! Flat builtins (see `hir::is_builtin` and `runtime::exec_builtin`):
//! `dt_now`, `dt_parse_iso`, `dt_format`, `dt_add`, `dt_diff_ms`,
//! `dt_parts`, `dt_parts_offset`, `dt_from_parts`,
//! `dt_local_offset_minutes`, `dt_is_leap`, `dt_days_in_month`.
//!
//! Uses the `chrono` crate for civil math (no timezone database).
//! Pure computation: all functions are allowed in
//! `klang run --sandbox` (only file-based compression follows the
//! temp-dir rule; datetime never touches the filesystem or network).
//!
//! Errors (never a panic on out-of-range input):
//! `E-TIME-PARSE` (unparseable ISO string),
//! `E-TIME-RANGE` (out-of-range date/time/offset/epoch),
//! `E-TIME-PATTERN` (bad `dt_format` pattern).
//! Wrong static/dynamic types are `E-TYPE` via the shared constructor.

use chrono::{Datelike, Timelike};
use chrono::{DateTime, Utc};

use crate::diagnostics::Diagnostic;
use crate::runtime::Value;

// ---------------------------------------------------------------------------
// Error constructors (every message names the offending value).
// ---------------------------------------------------------------------------

fn time_err(code: &str, message: String, cause: String, rule: &str) -> Diagnostic {
    Diagnostic::error(
        code,
        &message,
        "runtime",
        0,
        0,
        &cause,
        &["check the date, time and format string, then retry"],
        rule,
    )
}

fn parse_err(op: &str, input: &str) -> Diagnostic {
    let shown = redact(input);
    time_err(
        "E-TIME-PARSE",
        format!("{op}({shown}) failed: cannot parse as ISO-8601/RFC-3339"),
        format!("input {shown} is not a supported ISO datetime"),
        "time/parse",
    )
}

fn range_err(op: &str, detail: &str, cause: &str) -> Diagnostic {
    time_err(
        "E-TIME-RANGE",
        format!("{op} failed: {detail}"),
        cause.to_string(),
        "time/range",
    )
}

fn pattern_err(op: &str, pattern: &str, detail: &str) -> Diagnostic {
    let shown = redact(pattern);
    time_err(
        "E-TIME-PATTERN",
        format!("{op}({shown}) failed: {detail}"),
        detail.to_string(),
        "time/pattern",
    )
}

/// Redact long inputs past 200 chars (like A1 hex redaction).
fn redact(s: &str) -> String {
    if s.len() <= 200 {
        return format!("{s:?}");
    }
    let head = &s[..200];
    let rest = s.len() - 200;
    format!("{head:?}... [{rest} chars redacted]")
}

// ---------------------------------------------------------------------------
// Small helpers.
// ---------------------------------------------------------------------------

/// Epoch milliseconds for `dt` (UTC). `None` when `ms` is outside chrono's
/// representable civil range (i64 millis cover a wider span than chrono).
fn datetime_from_ms(ms: i64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp_millis(ms)
}

fn as_int(op: &str, v: &Value, what: &str) -> Result<i64, Diagnostic> {
    match v {
        Value::Int(n) => Ok(*n),
        other => Err(crate::runtime::type_err(op, what, other)),
    }
}

fn as_str(op: &str, v: &Value, what: &str) -> Result<String, Diagnostic> {
    match v {
        Value::Str(s) => Ok(s.clone()),
        other => Err(crate::runtime::type_err(op, what, other)),
    }
}

const WEEKDAY_ABBR: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const WEEKDAY_FULL: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTH_ABBR: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTH_FULL: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// True for Gregorian leap years (divisible by 4, not by 100 unless by 400).
pub fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Days in `month` (1..12) for `year`. `None` when `month` is out of range.
pub fn days_in_month_for(year: i64, month: i64) -> Option<i64> {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => Some(31),
        4 | 6 | 9 | 11 => Some(30),
        2 => Some(if is_leap_year(year) { 29 } else { 28 }),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Datetime builtins.
// ---------------------------------------------------------------------------

/// `dt_now() -> Int` (epoch milliseconds, UTC).
pub fn dt_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// `dt_parse_iso(s) -> Int` (epoch milliseconds, UTC).
///
/// Accepted (all interpreted as UTC when no offset is present):
/// RFC-3339 with `Z` or `±HH:MM` (e.g. `2024-02-29T12:34:56Z`,
/// `2024-02-29T12:34:56.123+00:00`), `YYYY-MM-DDTHH:MM:SS[.frac]`,
/// `YYYY-MM-DD HH:MM:SS[.frac]`, and date-only `YYYY-MM-DD` (midnight).
/// Anything else is `E-TIME-PARSE` naming the input.
pub fn dt_parse_iso(s: &str) -> Result<i64, Diagnostic> {
    const OP: &str = "dt_parse_iso";
    // 1. Strict RFC-3339 first (covers Z and numeric offsets).
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc).timestamp_millis());
    }
    // 2. Offset forms without the RFC-3339 colon (e.g. +0000).
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f%z",
        "%Y-%m-%d %H:%M:%S%.f%z",
        "%Y-%m-%dT%H:%M:%S%z",
        "%Y-%m-%d %H:%M:%S%z",
    ] {
        if let Ok(dt) = DateTime::parse_from_str(s, fmt) {
            return Ok(dt.with_timezone(&Utc).timestamp_millis());
        }
    }
    // 3. Naive UTC forms (no offset; assumed UTC).
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            return Ok(naive.and_utc().timestamp_millis());
        }
    }
    // 4. Date-only (midnight UTC).
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        if let Some(dt) = d.and_hms_opt(0, 0, 0) {
            return Ok(dt.and_utc().timestamp_millis());
        }
    }
    Err(parse_err(OP, s))
}

/// `dt_format(ms, pattern) -> Str` in UTC.
///
/// Strftime-style verbs: `%Y %m %d %H %M %S %f %z %a %A %b %B %j %p`
/// plus `%%` for a literal `%`. `%f` is milliseconds (`000`..`999`,
/// three digits); `%z` is always `+0000` (UTC); `%j` is the day of year
/// (`001`..`366`); `%a`/`%A` are English weekday names; `%b`/`%B` are
/// English month names; `%p` is `AM`/`PM`. Any other `%X` (or a trailing
/// lone `%`) is `E-TIME-PATTERN` naming the pattern. Out-of-chrono-range
/// `ms` is `E-TIME-RANGE`.
pub fn dt_format(ms: i64, pattern: &str) -> Result<String, Diagnostic> {
    const OP: &str = "dt_format";
    let dt = datetime_from_ms(ms).ok_or_else(|| {
        range_err(
            OP,
            &format!("epoch {ms} out of representable datetime range"),
            &format!("epoch {ms} has no UTC civil date"),
        )
    })?;
    let year = dt.year() as i64;
    let month = dt.month() as i64;
    let day = dt.day() as i64;
    let hour = dt.hour() as i64;
    let minute = dt.minute() as i64;
    let second = dt.second() as i64;
    let millis = dt.timestamp_subsec_millis() as i64;
    // 0=Sunday..6=Saturday (strftime %w); 1..366 ordinal (strftime %j).
    let weekday = dt.weekday().num_days_from_sunday() as i64;
    let yday = dt.ordinal() as i64;
    let mut out = String::with_capacity(pattern.len() + 16);
    let bytes = pattern.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        // '%' escape.
        i += 1;
        if i >= bytes.len() {
            return Err(pattern_err(OP, pattern, "trailing lone `%` in pattern"));
        }
        match bytes[i] as char {
            'Y' => out.push_str(&format!("{year:04}")),
            'm' => out.push_str(&format!("{month:02}")),
            'd' => out.push_str(&format!("{day:02}")),
            'H' => out.push_str(&format!("{hour:02}")),
            'M' => out.push_str(&format!("{minute:02}")),
            'S' => out.push_str(&format!("{second:02}")),
            'f' => out.push_str(&format!("{millis:03}")),
            'z' => out.push_str("+0000"),
            'a' => out.push_str(WEEKDAY_ABBR[weekday as usize]),
            'A' => out.push_str(WEEKDAY_FULL[weekday as usize]),
            'b' => out.push_str(MONTH_ABBR[(month - 1) as usize]),
            'B' => out.push_str(MONTH_FULL[(month - 1) as usize]),
            'j' => out.push_str(&format!("{yday:03}")),
            'p' => out.push_str(if hour < 12 { "AM" } else { "PM" }),
            '%' => out.push('%'),
            other => {
                return Err(pattern_err(
                    OP,
                    pattern,
                    &format!("unsupported verb `%{other}` (want %Y %m %d %H %M %S %f %z %a %A %b %B %j %p or %%)"),
                ));
            }
        }
        i += 1;
    }
    Ok(out)
}

/// `dt_add(ms, delta_map) -> Int` (UTC, DST-free).
///
/// `delta_map` keys (all optional, default 0, all `Int`):
/// `days`, `hours`, `minutes`, `seconds`, `ms` (milliseconds).
/// Unknown keys are ignored. Non-`Int` values are `E-TYPE`.
/// Overflow past `i64` is `E-TIME-RANGE`.
pub fn dt_add(ms: i64, delta: &Value) -> Result<i64, Diagnostic> {
    const OP: &str = "dt_add";
    let entries: &Vec<(String, Value)> = match delta {
        Value::Map(m) => m,
        other => return Err(crate::runtime::type_err(OP, "a map delta", other)),
    };
    let get = |key: &str| -> Result<i64, Diagnostic> {
        match entries.iter().find(|(k, _)| k == key) {
            None => Ok(0),
            Some((_, v)) => match v {
                Value::Int(n) => Ok(*n),
                other => Err(crate::runtime::type_err(
                    OP,
                    &format!("an int `{key}` delta"),
                    other,
                )),
            },
        }
    };
    let days = get("days")? as i128;
    let hours = get("hours")? as i128;
    let minutes = get("minutes")? as i128;
    let seconds = get("seconds")? as i128;
    let millis = get("ms")? as i128;
    let total: i128 = days * 86_400_000
        + hours * 3_600_000
        + minutes * 60_000
        + seconds * 1_000
        + millis;
    let result = ms as i128 + total;
    if result < i64::MIN as i128 || result > i64::MAX as i128 {
        return Err(range_err(
            OP,
            &format!("epoch {ms} plus delta overflows i64"),
            "datetime addition left the i64 epoch range",
        ));
    }
    Ok(result as i64)
}

/// `dt_diff_ms(a, b) -> Int` (`a - b`; positive when `a` is later).
/// Overflow is `E-TIME-RANGE`.
pub fn dt_diff_ms(a: i64, b: i64) -> Result<i64, Diagnostic> {
    const OP: &str = "dt_diff_ms";
    a.checked_sub(b).ok_or_else(|| {
        range_err(
            OP,
            &format!("diff of {a} and {b} overflows i64"),
            "datetime difference left the i64 range",
        )
    })
}

/// Shared parts-map builder (UTC fields of `dt`).
fn parts_map(dt: &DateTime<Utc>) -> Value {
    Value::Map(vec![
        ("year".to_string(), Value::Int(dt.year() as i64)),
        ("month".to_string(), Value::Int(dt.month() as i64)),
        ("day".to_string(), Value::Int(dt.day() as i64)),
        ("hour".to_string(), Value::Int(dt.hour() as i64)),
        ("minute".to_string(), Value::Int(dt.minute() as i64)),
        ("second".to_string(), Value::Int(dt.second() as i64)),
        (
            "millis".to_string(),
            Value::Int(dt.timestamp_subsec_millis() as i64),
        ),
        (
            "weekday".to_string(),
            Value::Int(dt.weekday().num_days_from_sunday() as i64),
        ),
        ("yday".to_string(), Value::Int(dt.ordinal() as i64)),
    ])
}

/// `dt_parts(ms) -> Map` in UTC:
/// `{year, month, day, hour, minute, second, millis, weekday, yday}`.
/// `weekday` is `0`=Sunday..`6`=Saturday; `yday` is `1`..`366`.
/// Out-of-chrono-range `ms` is `E-TIME-RANGE`.
pub fn dt_parts(ms: i64) -> Result<Value, Diagnostic> {
    const OP: &str = "dt_parts";
    let dt = datetime_from_ms(ms).ok_or_else(|| {
        range_err(
            OP,
            &format!("epoch {ms} out of representable datetime range"),
            &format!("epoch {ms} has no UTC civil date"),
        )
    })?;
    Ok(parts_map(&dt))
}

/// `dt_parts_offset(ms, offset_minutes) -> Map` (same shape as
/// `dt_parts`, but the wall clock at the fixed `offset_minutes` east of
/// UTC). DST-free: the offset is applied exactly. Valid offsets are
/// `-1080`..`+1080` (±18h, covering all real zones); anything else is
/// `E-TIME-RANGE`. Overflow is `E-TIME-RANGE`.
pub fn dt_parts_offset(ms: i64, offset_minutes: i64) -> Result<Value, Diagnostic> {
    const OP: &str = "dt_parts_offset";
    if offset_minutes < -1080 || offset_minutes > 1080 {
        return Err(range_err(
            OP,
            &format!("offset {offset_minutes} out of range -1080..1080"),
            "fixed offsets cover -18h..+18h",
        ));
    }
    let shifted = (ms as i128) + (offset_minutes as i128) * 60_000;
    if shifted < i64::MIN as i128 || shifted > i64::MAX as i128 {
        return Err(range_err(
            OP,
            &format!("epoch {ms} plus offset {offset_minutes} overflows i64"),
            "shifted instant left the i64 epoch range",
        ));
    }
    let dt = datetime_from_ms(shifted as i64).ok_or_else(|| {
        range_err(
            OP,
            &format!("epoch {ms} with offset {offset_minutes} out of range"),
            "shifted instant has no civil date",
        )
    })?;
    Ok(parts_map(&dt))
}

/// `dt_from_parts(year, month, day, hour, minute, second) -> Int` (UTC).
/// `year` `1`..`9999`, `month` `1`..`12`, `day` `1`..days-in-month
/// (so Feb 30 is `E-TIME-RANGE`), `hour` `0`..`23`, `minute`/`second`
/// `0`..`59`. Anything else is `E-TIME-RANGE`, never a panic.
pub fn dt_from_parts(
    year: i64,
    month: i64,
    day: i64,
    hour: i64,
    minute: i64,
    second: i64,
) -> Result<i64, Diagnostic> {
    const OP: &str = "dt_from_parts";
    if year < 1 || year > 9999 {
        return Err(range_err(
            OP,
            &format!("year {year} out of range 1..9999"),
            "years are 1..9999",
        ));
    }
    if !(1..=12).contains(&month) {
        return Err(range_err(
            OP,
            &format!("month {month} out of range 1..12"),
            "months are 1..12",
        ));
    }
    let dim = days_in_month_for(year, month).expect("month checked");
    if day < 1 || day > dim {
        return Err(range_err(
            OP,
            &format!("day {day} out of range 1..{dim} for {year}-{month:02}"),
            "day is outside the month",
        ));
    }
    if !(0..=23).contains(&hour) {
        return Err(range_err(
            OP,
            &format!("hour {hour} out of range 0..23"),
            "hours are 0..23",
        ));
    }
    if !(0..=59).contains(&minute) {
        return Err(range_err(
            OP,
            &format!("minute {minute} out of range 0..59"),
            "minutes are 0..59",
        ));
    }
    if !(0..=59).contains(&second) {
        return Err(range_err(
            OP,
            &format!("second {second} out of range 0..59"),
            "seconds are 0..59",
        ));
    }
    let date = chrono::NaiveDate::from_ymd_opt(year as i32, month as u32, day as u32)
        .ok_or_else(|| {
            range_err(
                OP,
                &format!("date {year}-{month:02}-{day:02} is not a calendar date"),
                "invalid calendar date",
            )
        })?;
    let naive = date
        .and_hms_opt(hour as u32, minute as u32, second as u32)
        .ok_or_else(|| {
            range_err(
                OP,
                &format!("time {hour:02}:{minute:02}:{second:02} is not a clock time"),
                "invalid clock time",
            )
        })?;
    Ok(naive.and_utc().timestamp_millis())
}

/// `dt_local_offset_minutes() -> Int`: the host OS local UTC offset in
/// minutes (e.g. `0` on UTC hosts). Pure `chrono::Local` read; infallible.
pub fn dt_local_offset_minutes() -> i64 {
    (chrono::Local::now().offset().local_minus_utc() / 60) as i64
}

/// `dt_is_leap(year) -> Bool` (`1` for leap years, else `0`).
/// Any `Int` year is accepted (no range error).
pub fn dt_is_leap(year: i64) -> i64 {
    i64::from(is_leap_year(year))
}

/// `dt_days_in_month(year, month) -> Int` (`28`..`31`).
/// `month` outside `1`..`12` is `E-TIME-RANGE`.
pub fn dt_days_in_month(year: i64, month: i64) -> Result<i64, Diagnostic> {
    const OP: &str = "dt_days_in_month";
    days_in_month_for(year, month).ok_or_else(|| {
        range_err(
            OP,
            &format!("month {month} out of range 1..12"),
            "months are 1..12",
        )
    })
}

// ---------------------------------------------------------------------------
// Value-level wrappers (called from `runtime::exec_builtin`).
// ---------------------------------------------------------------------------

/// Extract an `Int` operand or raise `E-TYPE` naming it.
pub fn int_operand(op: &str, v: &Value, what: &str) -> Result<i64, Diagnostic> {
    as_int(op, v, what)
}

/// Extract a `Str` operand or raise `E-TYPE` naming it.
pub fn str_operand(op: &str, v: &Value, what: &str) -> Result<String, Diagnostic> {
    as_str(op, v, what)
}
