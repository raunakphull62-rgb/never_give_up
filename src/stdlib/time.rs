//! Klang OS-interop time boundary (STDLIB-TIME-1).
//!
//! `time::sleep` / `time::now` / `time::elapsed` in the PRD map onto
//! Klang's flat builtins `time_sleep` / `time_now` / `time_elapsed`
//! (see `hir::is_builtin` and `runtime::exec_builtin`). Flat names are
//! deliberate: Klang's `::` call syntax only resolves through declared
//! `mod` blocks (`modules::rewrite_ctor` converts `m::f(args)` into a
//! `Call` only when `m` is a program module), so a namespaced
//! `time::sleep(...)` spelling would parse as an enum constructor and
//! fail type checking. Consistency with the existing builtins matters
//! more than the PRD's illustrative `::` shape.
//!
//! Clock choice: `now()` returns seconds since the Unix epoch via
//! `std::time::SystemTime` — a real wall-clock timestamp, not a
//! process-local tick count. There is no monotonic-clock precedent
//! elsewhere in this codebase to match (the only `Duration` use is the
//! MCP run timeout in `mcp.rs`); a Unix-epoch `f64` is directly usable
//! both as a timestamp and, paired with `elapsed()`, for measuring
//! durations. `elapsed(since)` is computed against the same clock as
//! `now()`, so the pair is self-consistent. All calls are synchronous
//! and blocking, consistent with `process::run`.
//!
//! All sleeping goes through the free functions here so both the
//! interpreter (`runtime::exec_builtin`) and unit tests share one error
//! mapping. Durations that cannot be slept — negative, `NaN`,
//! infinite, or too large to represent as a `Duration` — raise
//! `E-TIME-INVALID` with a distinct cause per failure mode (never one
//! shared generic message, cf. AUDIT F14).

use crate::diagnostics::Diagnostic;

/// `E-TIME-INVALID`: the requested sleep duration cannot be slept.
/// Each failure mode carries its own cause string.
fn time_invalid(op: &str, detail: &str, cause: &str) -> Diagnostic {
    Diagnostic::error(
        "E-TIME-INVALID",
        &format!("{op} failed: {detail}"),
        "runtime",
        0,
        0,
        cause,
        &["pass a finite duration in seconds >= 0"],
        "time/invalid",
    )
}

/// `E-TIME-INVALID` for a non-numeric runtime value where a duration
/// (or timestamp) was expected. Statically the builtins require
/// numbers, but dynamically-typed values (e.g. a string out of a map
/// lookup, which checks as `Unknown`) can still arrive here — a loud
/// diagnostic, never a silent sleep-0.
pub fn not_a_number(op: &str, what: &str, got: &str) -> Diagnostic {
    time_invalid(
        op,
        &format!("{what} needs a number, got `{got}`"),
        &format!("{what} received non-numeric value `{got}`"),
    )
}
///
/// Seconds since the Unix epoch as an `f64`.
///
/// Falls back to `0.0` only when the system clock is before the epoch
/// (i.e. `duration_since` fails); on any sane host this is the real
/// wall-clock time.
pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Block the calling thread for `seconds` seconds.
///
/// Fractional seconds are supported (`sleep(0.25)` sleeps a quarter of
/// a second — something the `run_process("sleep", ...)` workaround
/// cannot express). Fails with `E-TIME-INVALID` on negative, `NaN`,
/// infinite, or unrepresentably large inputs. `Duration::try_from_secs_f64`
/// (not `from_secs_f64`) is used so huge finite values are a loud
/// diagnostic, never a panic.
pub fn sleep(seconds: f64) -> Result<(), Diagnostic> {
    if seconds.is_nan() {
        return Err(time_invalid(
            "time_sleep",
            "sleep duration is NaN, which cannot be slept",
            "NaN is not a valid sleep duration",
        ));
    }
    if seconds.is_infinite() {
        return Err(time_invalid(
            "time_sleep",
            &format!("sleep duration {seconds} is infinite, which cannot be slept"),
            "infinite is not a valid sleep duration",
        ));
    }
    if seconds < 0.0 {
        return Err(time_invalid(
            "time_sleep",
            &format!("sleep duration {seconds} is negative"),
            &format!("negative duration {seconds} is not a valid sleep duration"),
        ));
    }
    let dur = std::time::Duration::try_from_secs_f64(seconds).map_err(|e| {
        time_invalid(
            "time_sleep",
            &format!("sleep duration {seconds} cannot be represented: {e}"),
            &format!("duration {seconds} overflows the representable range: {e}"),
        )
    })?;
    std::thread::sleep(dur);
    Ok(())
}

/// Seconds elapsed between a prior `now()` reading and the current
/// time, on the same clock. Positive when `since` came from `now()`.
///
/// A `since` value from the future (clock adjustment, or a fabricated
/// argument) yields a negative result rather than an error: clamping
/// would silently lie about the measurement.
pub fn elapsed(since: f64) -> f64 {
    now() - since
}

/// S6 `now_ms()`: milliseconds since the Unix epoch as an `i64`.
///
/// Stored in the language's `Int` (which is `i64` at runtime; values
/// far exceed the `i32` checked-arithmetic range, like `file_size`
/// past 2 GiB — they flow as values but `E-OVERFLOW` on arithmetic).
/// Falls back to `0` only when the clock is before the epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// S6 `now_iso()`: current UTC time as an RFC 3339 string
/// (`YYYY-MM-DDTHH:MM:SS.mmmZ`, e.g. `2026-10-10T04:35:00.123Z`).
/// Pure std (no chrono): civil date via Howard Hinnant's
/// days-to-civil algorithm over days since the Unix epoch.
pub fn now_iso() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (hour, minu, sec) = ((sod / 3_600) as u32, ((sod % 3_600) / 60) as u32, (sod % 60) as u32);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minu:02}:{sec:02}.{millis:03}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// S6 `sleep_ms(millis)`: block the calling thread for `millis`
/// milliseconds. `millis` must be finite and `>= 0`; negatives (and
/// `NaN`/infinite) are loud `E-TIME-INVALID`, never a silent no-op.
/// Uses `Duration::try_from_secs_f64(millis / 1000.0)` so huge finite
/// values are a diagnostic, never a panic. No timeout is added: the
/// caller decides the bound, exactly like `time_sleep`.
pub fn sleep_ms(millis: f64) -> Result<(), Diagnostic> {
    if millis.is_nan() {
        return Err(time_invalid(
            "sleep_ms",
            "sleep duration is NaN, which cannot be slept",
            "NaN is not a valid sleep duration",
        ));
    }
    if millis.is_infinite() {
        return Err(time_invalid(
            "sleep_ms",
            &format!("sleep duration {millis}ms is infinite, which cannot be slept"),
            "infinite is not a valid sleep duration",
        ));
    }
    if millis < 0.0 {
        return Err(time_invalid(
            "sleep_ms",
            &format!("sleep duration {millis}ms is negative"),
            &format!("negative duration {millis}ms is not a valid sleep duration"),
        ));
    }
    let dur =
        std::time::Duration::try_from_secs_f64(millis / 1000.0).map_err(|e| {
            time_invalid(
                "sleep_ms",
                &format!("sleep duration {millis}ms cannot be represented: {e}"),
                &format!("duration {millis}ms overflows the representable range: {e}"),
            )
        })?;
    std::thread::sleep(dur);
    Ok(())
}
