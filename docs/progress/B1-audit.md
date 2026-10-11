# B1 stdlib audit — what exists before the gap fill

Branch: `agent2`. Baseline `cargo test --locked`: running (see `/tmp/b1.log`);
expected 870 + S9 tests per the PRD. This file is written BEFORE any gap-fill
code, per Lane B stage B1.

Conventions: "native" = Rust builtin (`src/hir.rs` `is_builtin` +
`src/runtime/mod.rs` `exec_builtin`, helpers under `src/stdlib/`).
"package" = pure-Klang package under `stdlib-packages/` (tested by its own
`*_test.klang`, pinned in `tests/parity/allowlist.toml` as stdlib self-tests).
"method" = `base.method()` dispatch in `runtime::exec_method`.

Shadowing constraint (rules the gap-fill design): a user `fn` may not reuse a
native builtin name (`W-SHADOW`/`E-DUPLICATE` via `is_builtin`). Every package
function name below is therefore RESERVED against a same-named native builtin:
adding e.g. native `json_parse` would break `stdlib-packages/json` (and its
parity self-test). Native additions in B1 use only names no package uses.

## 2.5 table (needed for complex programs)

| Needed item | Exists? | Where | Tested? | B1 action |
|---|---|---|---|---|
| JSON parse, clear error on bad input | YES (package) | `stdlib-packages/json/src/parse.klang`: `json_parse(text)->map{ok,value,error}` with `line/col` errors; `json_parse_or_assert` | YES (`json_test.klang`, parity self-test) | keep; no native duplicate (would shadow `json_parse`) |
| JSON stringify | PARTIAL (package) | `encode.klang`: `json_quote/json_int/json_str/json_bool/json_obj1-3/json_arr2`; `json_pretty(v,indent)` stringifies any parsed value | YES (`json_test.klang`) | keep; document as the C1 story (no single `json_stringify(v)`; `json_pretty(v,0)` covers it) |
| JSON pretty-print option | YES (package) | `json_pretty(v, indent)` | YES | keep |
| regex match | YES (native+package) | native `regex_is_match(pat,text)->bool` (`src/stdlib/regex.rs`, `E-REGEX-INVALID-PATTERN`); package `rx_match/regex_matches` | YES (`osio_regex_gates`, `regex_test.klang`) | keep |
| regex find_all | NO | — | — | ADD native `regex_find_all(pat,text)->array<str>` |
| regex replace | NO | — | — | ADD native `regex_replace(pat,text,repl)->str` (`$1`/`$name` expansion) |
| regex captures | YES (native+package) | native `regex_find` returns `{matched,match,groups,named}`; package `regex_groups/regex_named_map` | YES | keep; `regex_find_all` intentionally returns flat matches only (captures per match would need nested maps; documented) |
| CSV parse | YES (package) | `stdlib-packages/csv/src/table.klang`: `csv_parse(text,delim)`, `csv_parse_comma` (quotes, `""` escapes, CRLF) | YES (`csv_test.klang`, parity) | keep; no native duplicate |
| CSV write | YES (package) | `csv_write(rows,delim)`, `csv_write_row`, `csv_write_comma`, `csv_escape_field` | YES | keep |
| base64 | YES (package) | `stdlib-packages/base64/src/codec.klang`: `base64_encode(s)->str`, `base64_decode(s)->map{ok,value,error}` over UTF-8 bytes | YES (`base64_test.klang`, parity) | keep; no native duplicate (binary-safe strategy for §5, documented) |
| hex | YES (package) | `stdlib-packages/hex/src/codec.klang`: `hex_encode`, `hex_decode->map` | YES (`hex_test.klang`, parity) | keep |
| sha256 | YES (native+package) | native `sha256(s)->hex`, `hmac_sha256(k,m)` (`sha2`+`hmac` crates); package `crypto_sha256` (teaching wrapper) | YES (`floatmath`/crypto gates, `crypto_test.klang`) | keep |
| random (seeded) | PARTIAL | native `random_int(lo,hi)`/`random_float()` are OS-seeded (explicitly NOT reproducible); package `random`: `rand_new(seed)`, `rand_int/rand_float/rand_choice/rand_shuffle` with explicit state (deterministic, Park-Miller) | YES (both) | ADD native `random_seed(seed)`, `random_seeded_int(lo,hi)`, `random_seeded_float()` — global deterministic RNG for programs that want `rand()`-style reproducibility without threading state; package stays the explicit-state story |
| uuid v4 | NO | — | — | ADD native `uuid_v4()->str` (122-bit, OS entropy + SplitMix64 fallback, same source as `random_int`) |
| path join | YES (package) | `path_join(a,b)` (pure `/` semantics) | YES (`path_test.klang`, parity) | keep |
| path dirname | YES (package) | `path_dirname` | YES | keep |
| path basename | YES (package) | `path_basename` | YES | keep |
| path extension | YES (package) | `path_extension` (+`path_stem`) | YES | keep |
| path normalize | YES (package) | `path_normalize` (lexical `.`/`..`) | YES | keep |
| path absolute | NO (package has only `path_is_absolute` predicate) | — | — | ADD native `path_absolute(p)->str` (cwd join + lexical normalize, no FS touch so it works for not-yet-existing paths; infallible) |
| string trim | YES (method) | `"s".trim()` | YES | keep |
| string split | YES (method) | `"s".split(d)` | YES | keep |
| string join | YES (method) | `arr.join(sep)` | YES | keep |
| string replace | YES (method) | `"s".replace(a,b)` | YES | keep |
| string starts_with/ends_with | YES (method) | both | YES | keep |
| string pad | PARTIAL (package only) | `text_pad_left/text_pad_right/text_center` (overshoot multi-char pads; silent on negatives) | YES (`text_test.klang`) | ADD width-exact char-based methods `"s".pad_start(w,pad)`, `"s".pad_end(w,pad)` (loud on empty pad/negative/huge; documented deviation from package overshoot) |
| string repeat | PARTIAL (package only) | `str_repeat/text_repeat` (silent on negatives, unbounded) | YES | ADD `"s".repeat(n)` method (loud on negative, 1M-char cap) |
| string lines | PARTIAL (package only) | `str_lines` (bare `\n` split, keeps `\r`) | YES | ADD `"s".lines()` method (strips one trailing `\r` per line, CRLF-safe) |
| string to_upper/lower | YES (method) | `"s".upper()/lower()` | YES | keep |
| string format with placeholders | NO (`format(...)` joins with spaces, no placeholders) | — | — | ADD native `sprintf(fmt,args_array)` (`{}` in order, `{{`/`}}` escapes; `E-FMT-INVALID`/`E-FMT-ARG-MISMATCH`) |
| file read_file | YES (native+package `fs_read`) | `read_file` (`E-IO-*`) | YES | keep |
| file write_file | YES | `write_file` | YES | keep |
| file append_file | YES | `append_file` | YES | keep |
| file exists | YES | `exists` | YES | keep |
| file is_dir | YES | `is_dir` | YES | keep |
| file list_dir | YES | `list_dir` (sorted) | YES | keep |
| file make_dirs | YES (+`make_dir`) | `make_dirs`/`make_dir` | YES | keep |
| file remove_file | YES | `remove_file` | YES | keep |
| file rename_file | YES (+`copy_file`) | `rename_file` | YES | keep |
| file stat size | YES | `file_size` | YES | keep |
| file stat mtime | NO | — | — | ADD native `file_mtime(path)->f64` (secs since epoch, `E-IO-*` via shared `map_io_error`, same `reject_unsafe_path` guard) |
| env/args | YES (native+package `os_*`) | `env/get_env/set_env/args/cwd` | YES | keep |
| exit codes | YES | `exit(code)` (`E-EXIT`, uncatchable) | YES | keep |
| printf-style formatter | NO (see sprintf row) | — | — | ADD `sprintf` (this is the same gap) |

## Section 5 language gaps (audit, add only if a capstone needs it)

| Gap | Exists? | Notes | Capstone need? | B1 action |
|---|---|---|---|---|
| bytes type / binary-safe strategy | NO bytes type; strategy EXISTS via base64/hex packages + lossy-UTF-8 transport convention (`process::run`, `http` bodies) | C1/C4 move bytes as base64/hex strings or lossy text; `sha256` hex for checksums | C4 checksums via `sha256`; archives via B3 (not B1) | document; no bytes type in B1 |
| maps iteration order | DOCUMENTED (insertion order; `keys()` returns it; int index is positional) | `BUILTINS.md` + `index_value` positional rule | C1 `GET /list` last-20 relies on it | keep; no change |
| array map/filter/reduce/any/all/find/contains/reverse/sort/index_of/push/pop/extend/slice | PARTIAL: `contains/join/push/pop/insert/len` methods + flat `push/pop/insert/range/sort/slice/len/keys` exist; `map/filter/reduce/any/all/find/reverse/index_of/extend` MISSING | need closures-as-values plumbing for the higher-order set | NOT needed by C1 (needs HTTP+SQL+JSON) or C4 (needs B3+A3) | report only; no B1 implementation |
| string indexing chars vs bytes (S7 issue) | MIXED, documented: `len()`/`"s".len()` are BYTES (`len("é")==2`); `s[i]`, `chars()`, `slice()` are CHARS | `BUILTINS.md` states it; `slice_str`/`index_value` are char-based | C1/C4 work on ASCII logs/JSON; non-ASCII safe via `chars()` | document; no behavior change in B1 (changing `len` would break existing gates) |
| closures capturing correctly + first-class fn values to sort_by et al | YES closures with by-value snapshot (`ClosureNew`, dynamic `Call` dispatch); NO `sort_by` parameter | `closure_gates` cover capture | neither capstone needs `sort_by` | report only |
| optional/null convention | NO null type; convention EXISTS: `""` for missing env/stdin-EOF, `0/1` bool encoding, `{ok,value,error}` maps for json/csv/base64/hex/url | consistent across packages | C1 404/400 are HTTP statuses + error JSON, not nulls | document; no null type in B1 |
| int parse/format hex/binary | MISSING (`parse_int` strict decimal i32; `int()` decimal i64; no radix; no hex formatting) | — | C1/C4 need decimal only | report only |
| float formatting with precision | MISSING (`str(3.0)=="3.0"`, no precision control) | C4 summary table can use `sprintf` with default rendering in B1; real precision formatting is NOT in B1 | nice-to-have for C4 tables | `sprintf` renders like `str()` (no precision spec); report precision as still-missing |
| multi-line strings | MISSING (no `"""` literals; `"a\nb"` escapes + `lines()` cover it) | check parser: no triple-quote form | C1/C4 fixtures use `\n` escapes | report only |

## What B1 builds (summary)

Native, all in ONE new file `src/stdlib/b1.rs` (one `mod` line; every other
touch is an appended registration arm):
`regex_find_all`, `regex_replace`, `uuid_v4`, `path_absolute`, `file_mtime`,
`sprintf`, `random_seed`, `random_seeded_int`, `random_seeded_float`
(9 flat builtins) + `"s".pad_start/pad_end/repeat/lines` (4 methods).
New error codes: `E-FMT-INVALID`, `E-FMT-ARG-MISMATCH`, `E-STR-INVALID`,
`E-STR-TOO-LARGE`, `E-RANDOM-BOUNDS`, `E-REGEX-TOO-MANY`, `E-REGEX-TOO-LARGE`
(`E-REGEX-INVALID-PATTERN` and `E-IO-*` reused).
Deliberately NOT built: native JSON/CSV/base64/hex/path-join duplicates
(shadow-collision with tested packages; packages already satisfy C1/C4).
