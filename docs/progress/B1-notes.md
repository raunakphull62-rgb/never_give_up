# B1 notes — proposed text for shared docs (stage B8 merges)

## For `docs/reference.md` (new builtins section)

Add after the `sort` row:

- `regex_find_all(pat, text) -> array<str>`: all non-overlapping matches in
  order; `[]` when nothing matches. Bad pattern is
  `E-REGEX-INVALID-PATTERN`; over 10,000 matches is `E-REGEX-TOO-MANY`.
- `regex_replace(pat, text, repl) -> str`: replaces every match; `repl`
  supports `$1`, `$name`, `$$` (the `regex`-crate expansion). Output over
  1,000,000 bytes is `E-REGEX-TOO-LARGE`.
- `uuid_v4() -> str`: random UUID v4 (`xxxxxxxx-xxxx-4xxx-yxxx-…`).
  OS entropy with a deterministic fallback; not for security tokens on
  hosts without `/dev/urandom` (same caveat as `random_int`).
- `path_absolute(path) -> str`: CWD-joined, lexically normalized absolute
  path. Never touches the filesystem (works for not-yet-existing paths);
  never fails. Separators are platform-native; the `path_*` package helpers
  stay for portable `/`-only string work.
- `file_mtime(path) -> f64`: modification time, seconds since the Unix epoch
  (same clock as `time_now()`). Missing → `E-IO-NOT-FOUND`, etc., via the
  shared file-error mapping; unsafe paths are `E-RUNTIME`.
- `sprintf(fmt, args) -> str`: `{}` takes the next arg (rendered like
  `str()`); `{{`/`}}` are literal braces. Extra args are ignored; fewer args
  than placeholders is `E-FMT-ARG-MISMATCH`; a lone brace is `E-FMT-INVALID`.
- `random_seed(seed) -> i32`, `random_seeded_int(lo, hi) -> i32`,
  `random_seeded_float() -> f64`: deterministic global RNG (SplitMix64).
  Same seed replays the same sequence. `lo > hi` is `E-RANDOM-BOUNDS`. Not
  for keys/tokens (deterministic by design); the `random` package stays the
  explicit-state story.

Add to the string-methods list: `pad_start(width, pad)` / `pad_end(width,
pad)` (width-exact, char-based; empty pad or negative width is
`E-STR-INVALID`, output over 1,000,000 chars is `E-STR-TOO-LARGE`),
`repeat(n)` (same codes), `lines()` (CRLF-safe split, infallible).

## For `docs/limitations.md`

- No bytes type in v1.0 scope: move binary data as base64/hex strings (the
  `base64`/`hex` packages) or lossy-UTF-8 text (same convention as
  `process::run` stdout and HTTP bodies).
- `len()` and `"s".len()` count BYTES; `s[i]`, `chars()`, `slice()`, the B1
  pads/`repeat`/`lines` count CHARS. Use `chars()` for non-ASCII work.
- `text_pad_left`/`text_pad_right` (packages) repeat the whole pad and can
  overshoot the width with multi-char pads; the native `pad_start`/`pad_end`
  are width-exact. `text_repeat`/`str_repeat` return `""` for negative
  counts; native `repeat` is `E-STR-INVALID` instead.
- `sprintf` renders args like `str()` (no float-precision spec); precision
  formatting is out of scope for v1.0.
- The seeded RNG is process-global and deterministic; OS-seeded
  `random_int`/`random_float` stay independent of it.

## For `docs/LANGUAGE_STATUS.md` / `SPEC.md` (one line each)

- B1: stdlib audit complete; gap fill adds `regex_find_all`,
  `regex_replace`, `uuid_v4`, `path_absolute`, `file_mtime`, `sprintf`,
  `random_seed`/`random_seeded_int`/`random_seeded_float`,
  `pad_start`/`pad_end`/`repeat`/`lines` (all tested in `tests/b1_gates.rs`).
