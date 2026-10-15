//! L1 datetime + compression gates (Wave 2 Stage L1).
//!
//! No network, no fixed ports, no fixed `/tmp` names (unique temp dirs per
//! test, removed afterwards). String 10 MB round trips go through direct
//! stdlib calls plus `read_file`-fed Klang strings because the `repeat`
//! helper caps at 1M chars; file 10 MB round trips use Rust-created files
//! plus Klang `gzip_file`/`gunzip_file`. All hostile inputs must be loud
//! diagnostics, never a host panic. Sandbox-flag tests serialize on a mutex
//! because the flag is process-global.

use std::collections::HashMap;

use klang::parser::Parser;

static L1_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn parse_ok(src: &str) -> klang::ast::Program {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    assert!(
        klang::hir::TypedHIR::check(prog.clone()).is_ok(),
        "must check clean: {src}"
    );
    prog
}

fn run_ok(src: &str) -> (i32, Vec<String>) {
    let _guard = L1_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let r = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("runs");
    klang::sandbox::set_sandbox(false);
    r
}

fn run_err(src: &str) -> klang::diagnostics::Diagnostic {
    let _guard = L1_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let e = klang::runtime::run_with_output_value(&mir, "main", &[], &HashMap::new())
        .expect_err("must fail");
    klang::sandbox::set_sandbox(false);
    e
}

fn check_err_code(src: &str, want: &str) {
    let mut p = Parser::new(src);
    let prog = p.parse_program().expect("parses");
    let err = klang::hir::TypedHIR::check(prog).expect_err("must fail check");
    let codes: Vec<_> = err.iter().map(|d| d.code.as_str()).collect();
    assert!(codes.contains(&want), "want {want} in {codes:?}: {src}");
}

fn run_sandboxed(src: &str) -> Result<(i32, Vec<String>), String> {
    let _guard = L1_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(true);
    let prog = parse_ok(src);
    let mir = klang::mir::lower(&prog);
    let r = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new());
    klang::sandbox::set_sandbox(false);
    match r {
        Ok(v) => Ok(v),
        Err(d) => Err(format!("{} {}", d.code, d.to_json())),
    }
}

fn mktemp_dir(prefix: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    p.push(format!(
        "l1_{}_{}_{}",
        prefix,
        std::process::id(),
        nanos % 1_000_000_000
    ));
    std::fs::create_dir_all(&p).expect("mktemp");
    p
}

fn klang_bin() -> String {
    env!("CARGO_BIN_EXE_klang").to_string()
}

// ---- datetime happy paths ----

#[test]
fn l1_dt_now_and_known_epochs() {
    let src = "fn main() -> i32 { let n = dt_now() print(n > 0) print(dt_parse_iso(\"1970-01-01T00:00:00Z\")) print(dt_parse_iso(\"1970-01-01\")) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "0", "0"]);
}

#[test]
fn l1_dt_format_all_verbs() {
    // 2024-02-29T12:34:56.789Z: Thursday, day 60 of leap year 2024.
    let src = "fn main() -> i32 { let ms = dt_from_parts(2024, 2, 29, 12, 34, 56) print(dt_format(ms, \"%Y-%m-%d\")) print(dt_format(ms, \"%H:%M:%S\")) print(dt_format(ms, \"%a\")) print(dt_format(ms, \"%A\")) print(dt_format(ms, \"%b\")) print(dt_format(ms, \"%B\")) print(dt_format(ms, \"%j\")) print(dt_format(ms, \"%p\")) print(dt_format(ms, \"%z\")) print(dt_format(ms, \"100%%\")) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(
        out,
        vec![
            "2024-02-29",
            "12:34:56",
            "Thu",
            "Thursday",
            "Feb",
            "February",
            "060",
            "PM",
            "+0000",
            "100%"
        ]
    );
}

#[test]
fn l1_dt_format_millis_and_midnight() {
    let src = "fn main() -> i32 { let ms = dt_parse_iso(\"2024-02-29T12:34:56.789Z\") print(dt_format(ms, \"%S.%f\")) print(dt_format(0, \"%Y-%m-%d %H:%M:%S\")) print(dt_format(0, \"%a %j %p\")) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["56.789", "1970-01-01 00:00:00", "Thu 001 AM"]);
}

#[test]
fn l1_dt_parse_format_round_trip_50() {
    // 50 valid timestamps incl. leap days; second-precision ISO round trip.
    let _guard = L1_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    let mut cases: Vec<(i64, i64, i64, i64, i64, i64)> = vec![
        (2024, 2, 29, 12, 34, 56),
        (2000, 2, 29, 0, 0, 0),
        (2020, 2, 29, 23, 59, 59),
        (1970, 1, 1, 0, 0, 0),
        (1969, 12, 31, 23, 59, 59),
        (1999, 12, 31, 23, 59, 59),
        (2000, 1, 1, 0, 0, 0),
    ];
    for i in 0..43 {
        let year = 1971 + (i * 7) % 55;
        let month = (i % 12) + 1;
        let day = ((i * 5) % 28) + 1;
        let hour = (i * 3) % 24;
        let minute = (i * 13) % 60;
        let second = (i * 29) % 60;
        cases.push((year, month, day, hour, minute, second));
    }
    assert_eq!(cases.len(), 50);
    for (y, mo, d, h, mi, s) in &cases {
        let ms = klang::stdlib::l1_datetime::dt_from_parts(*y, *mo, *d, *h, *mi, *s)
            .expect("valid parts");
        let iso = klang::stdlib::l1_datetime::dt_format(ms, "%Y-%m-%dT%H:%M:%S")
            .expect("formats");
        let back =
            klang::stdlib::l1_datetime::dt_parse_iso(&iso).expect("parses back");
        assert_eq!(ms, back, "round trip {iso}");
    }
    klang::sandbox::set_sandbox(false);
}

#[test]
fn l1_dt_add_diff_utc_math_dst_free() {
    // US DST spring-forward 2024-03-10: UTC math stays exactly 24h.
    let src = "fn main() -> i32 { let a = dt_parse_iso(\"2024-03-09T12:00:00Z\") let b = dt_add(a, {\"days\": 1}) print(dt_format(b, \"%Y-%m-%dT%H:%M:%S\")) print(dt_diff_ms(b, a)) print(dt_diff_ms(a, b)) let c = dt_add(a, {\"hours\": 2, \"minutes\": 30, \"seconds\": 10, \"ms\": 500}) print(dt_diff_ms(c, a)) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(
        out,
        vec!["2024-03-10T12:00:00", "86400000", "-86400000", "9010500"]
    );
}

#[test]
fn l1_dt_parts_and_offset() {
    let src = "fn main() -> i32 { let p = dt_parts(0) print(p[\"year\"]) print(p[\"month\"]) print(p[\"day\"]) print(p[\"hour\"]) print(p[\"weekday\"]) print(p[\"yday\"]) let q = dt_parts_offset(0, 60) print(q[\"hour\"]) print(q[\"day\"]) let r = dt_parts_offset(0, -60) print(r[\"hour\"]) print(r[\"day\"]) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(
        out,
        vec!["1970", "1", "1", "0", "4", "1", "1", "1", "23", "31"]
    );
}

#[test]
fn l1_dt_from_parts_leap_rules() {
    let src = "fn main() -> i32 { print(dt_from_parts(2024, 2, 29, 0, 0, 0) > 0) print(dt_from_parts(2000, 2, 29, 0, 0, 0) > 0) print(dt_from_parts(2023, 2, 28, 0, 0, 0) > 0) print(dt_is_leap(2024)) print(dt_is_leap(1900)) print(dt_is_leap(2000)) print(dt_is_leap(2023)) print(dt_days_in_month(2024, 2)) print(dt_days_in_month(2023, 2)) print(dt_days_in_month(2024, 4)) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(
        out,
        vec!["1", "1", "1", "1", "0", "1", "0", "29", "28", "30"]
    );
}

#[test]
fn l1_dt_local_offset_in_range() {
    let src = "fn main() -> i32 { let o = dt_local_offset_minutes() print(o >= -1080) print(o <= 1080) return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "1"]);
}

// ---- datetime errors ----

#[test]
fn l1_dt_err_parse() {
    for src in [
        "fn main() -> i32 { print(dt_parse_iso(\"not-a-date\")) return 0 }",
        "fn main() -> i32 { print(dt_parse_iso(\"\")) return 0 }",
        "fn main() -> i32 { print(dt_parse_iso(\"2024-13-01\")) return 0 }",
        "fn main() -> i32 { print(dt_parse_iso(\"2024-02-30T00:00:00Z\")) return 0 }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-TIME-PARSE", "{src}: {}", err.to_json());
    }
}

#[test]
fn l1_dt_err_range() {
    for src in [
        "fn main() -> i32 { print(dt_from_parts(2023, 2, 30, 0, 0, 0)) return 0 }",
        "fn main() -> i32 { print(dt_from_parts(2024, 13, 1, 0, 0, 0)) return 0 }",
        "fn main() -> i32 { print(dt_from_parts(2024, 1, 1, 25, 0, 0)) return 0 }",
        "fn main() -> i32 { print(dt_from_parts(0, 1, 1, 0, 0, 0)) return 0 }",
        "fn main() -> i32 { print(dt_days_in_month(2024, 13)) return 0 }",
        "fn main() -> i32 { print(dt_parts_offset(0, 2000)) return 0 }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-TIME-RANGE", "{src}: {}", err.to_json());
    }
}

#[test]
fn l1_dt_err_pattern() {
    for src in [
        "fn main() -> i32 { print(dt_format(0, \"%Q\")) return 0 }",
        "fn main() -> i32 { print(dt_format(0, \"abc%\")) return 0 }",
        "fn main() -> i32 { print(dt_format(0, \"%c\")) return 0 }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-TIME-PATTERN", "{src}: {}", err.to_json());
    }
}

// ---- compression happy paths ----

#[test]
fn l1_gzip_string_round_trip_small() {
    let src = "fn main() -> i32 { print(gzip_decompress(gzip_compress(\"\")) == \"\") print(gzip_decompress(gzip_compress(\"x\")) == \"x\") print(gzip_decompress(gzip_compress(\"hello world, hello world\")) == \"hello world, hello world\") return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "1", "1"]);
}

#[test]
fn l1_gzip_string_10mb_direct_and_klang() {
    // 10 MB highly-compressible string: direct stdlib round trip (avoids
    // the 1M-char `repeat` cap) plus a Klang-level check via read_file.
    let big = "ab".repeat(5 * 1024 * 1024);
    assert_eq!(big.len(), 10 * 1024 * 1024);
    let hex = klang::stdlib::l1_compress::gzip_compress(&big).expect("compress 10mb");
    assert!(hex.starts_with("1f8b08"), "gzip magic");
    let back = klang::stdlib::l1_compress::gzip_decompress(&hex).expect("decompress");
    assert_eq!(back, big);
    // Klang-level: write via Rust, read + compress + decompress in Klang.
    let dir = mktemp_dir("gz10m");
    let src_path = dir.join("big.txt");
    std::fs::write(&src_path, big.as_bytes()).expect("write big");
    let sp = src_path.to_string_lossy().into_owned();
    let src = format!("fn main() -> i32 {{ let s = read_file(\"{sp}\") print(len(s) == 10485760) let g = gzip_compress(s) print(gzip_decompress(g) == s) return 42 }}");
    let (v, out) = run_ok(&src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "1"]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn l1_gzip_known_vector_and_magic() {
    // Python gzip.compress(b'hello world') interop vector.
    let hex = "1f8b080072afcf6a02ffcb48cdc9c95728cf2fca49010085114a0d0b000000";
    let back = klang::stdlib::l1_compress::gzip_decompress(hex).expect("known vector");
    assert_eq!(back, "hello world");
    let src = "fn main() -> i32 { let g = gzip_compress(\"hi\") print(slice(g, 0, 6) == \"1f8b08\") print(gzip_decompress(\"1f8b080072afcf6a02ffcb48cdc9c95728cf2fca49010085114a0d0b000000\") == \"hello world\") return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "1"]);
}

#[test]
fn l1_zlib_round_trip_and_known_vector() {
    let hex = "789ccb48cdc9c95728cf2fca4901001a0b045d";
    let back = klang::stdlib::l1_compress::zlib_decompress(hex).expect("zlib vector");
    assert_eq!(back, "hello world");
    let src = "fn main() -> i32 { print(zlib_decompress(zlib_compress(\"\")) == \"\") print(zlib_decompress(zlib_compress(\"abc-123\")) == \"abc-123\") print(zlib_decompress(\"789ccb48cdc9c95728cf2fca4901001a0b045d\") == \"hello world\") return 42 }";
    let (v, out) = run_ok(src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "1", "1"]);
}

#[test]
fn l1_file_round_trips_0_1_bytes() {
    let dir = mktemp_dir("f01");
    let d = dir.to_string_lossy().into_owned();
    let src = format!("fn main() -> i32 {{ write_file(\"{d}/e.txt\", \"\") print(gzip_file(\"{d}/e.txt\", \"{d}/e.gz\") > 0) print(gunzip_file(\"{d}/e.gz\", \"{d}/e.out\") == 0) print(read_file(\"{d}/e.out\") == \"\") write_file(\"{d}/o.txt\", \"Z\") print(gzip_file(\"{d}/o.txt\", \"{d}/o.gz\") > 0) print(gunzip_file(\"{d}/o.gz\", \"{d}/o.out\") == 1) print(read_file(\"{d}/o.out\") == \"Z\") print(zlib_file(\"{d}/o.txt\", \"{d}/o.zz\") > 0) print(unzlib_file(\"{d}/o.zz\", \"{d}/o.zout\") == 1) print(read_file(\"{d}/o.zout\") == \"Z\") return 42 }}");
    let (v, out) = run_ok(&src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "1", "1", "1", "1", "1", "1", "1", "1"]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn l1_gzip_file_10mb_round_trip() {
    let dir = mktemp_dir("f10m");
    let d = dir.to_string_lossy().into_owned();
    let big = "0123456789abcdef".repeat(655360);
    assert_eq!(big.len(), 10 * 1024 * 1024);
    std::fs::write(dir.join("big.bin"), big.as_bytes()).expect("write big");
    let src = format!("fn main() -> i32 {{ print(file_size(\"{d}/big.bin\") == 10485760) print(gzip_file(\"{d}/big.bin\", \"{d}/big.gz\") > 0) print(gunzip_file(\"{d}/big.gz\", \"{d}/big.out\") == 10485760) return 42 }}");
    let (v, out) = run_ok(&src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "1", "1"]);
    let back = std::fs::read(dir.join("big.out")).expect("read back");
    assert_eq!(back, big.as_bytes());
    std::fs::remove_dir_all(&dir).ok();
}

// ---- compression errors ----

#[test]
fn l1_err_compress_corrupt() {
    for src in [
        "fn main() -> i32 { print(gzip_decompress(\"abc\")) return 0 }",
        "fn main() -> i32 { print(gzip_decompress(\"zz\")) return 0 }",
        "fn main() -> i32 { print(gzip_decompress(\"00ff\")) return 0 }",
        "fn main() -> i32 { print(zlib_decompress(\"00ff\")) return 0 }",
        "fn main() -> i32 { print(zlib_decompress(\"abc\")) return 0 }",
    ] {
        let err = run_err(src);
        assert_eq!(err.code, "E-COMPRESS-CORRUPT", "{src}: {}", err.to_json());
    }
    // Corrupt file content.
    let dir = mktemp_dir("corrupt");
    let d = dir.to_string_lossy().into_owned();
    std::fs::write(dir.join("bad.gz"), b"not-gzip-at-all!!!!!!!!").expect("write");
    let src = format!("fn main() -> i32 {{ print(gunzip_file(\"{d}/bad.gz\", \"{d}/out\")) return 0 }}");
    let err = run_err(&src);
    assert_eq!(err.code, "E-COMPRESS-CORRUPT", "{}", err.to_json());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn l1_err_compress_too_large_via_tar_limit() {
    let dir = mktemp_dir("toolarge");
    let d = dir.to_string_lossy().into_owned();
    std::fs::create_dir_all(dir.join("src")).expect("mkdir");
    std::fs::write(dir.join("src/a.txt"), vec![b'A'; 2048]).expect("write");
    let src = format!("fn main() -> i32 {{ print(tar_create(\"{d}/src\", \"{d}/a.tar\")) print(tar_extract_limit(\"{d}/a.tar\", \"{d}/out\", 100)) return 0 }}");
    let err = run_err(&src);
    assert_eq!(err.code, "E-COMPRESS-TOO-LARGE", "{}", err.to_json());
    // Bad limit itself is also TOO-LARGE.
    let src2 = format!("fn main() -> i32 {{ print(tar_extract_limit(\"{d}/a.tar\", \"{d}/out2\", 0 - 5)) return 0 }}");
    let err2 = run_err(&src2);
    assert_eq!(err2.code, "E-COMPRESS-TOO-LARGE", "{}", err2.to_json());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn l1_tar_round_trip_nested_unusual_names() {
    let dir = mktemp_dir("tar");
    let d = dir.to_string_lossy().into_owned();
    std::fs::create_dir_all(dir.join("src/nested/deep")).expect("mkdir");
    std::fs::write(dir.join("src/top.txt"), b"top").expect("w");
    std::fs::write(dir.join("src/nested/mid.txt"), b"mid").expect("w");
    std::fs::write(dir.join("src/nested/deep/leaf.txt"), b"leaf").expect("w");
    std::fs::write(dir.join("src/with space.txt"), b"sp").expect("w");
    std::fs::write(dir.join("src/ünicode-ß.txt"), "uni".as_bytes()).expect("w");
    std::fs::write(dir.join("src/empty.txt"), b"").expect("w");
    let src = format!("fn main() -> i32 {{ print(tar_create(\"{d}/src\", \"{d}/a.tar\")) print(tar_extract(\"{d}/a.tar\", \"{d}/out\")) print(read_file(\"{d}/out/./top.txt\")) print(read_file(\"{d}/out/./nested/deep/leaf.txt\")) print(read_file(\"{d}/out/./with space.txt\")) print(read_file(\"{d}/out/./empty.txt\") == \"\") return 42 }}");
    let (v, out) = run_ok(&src);
    assert_eq!(v, 42, "prints: {out:?}");
    assert_eq!(out, vec!["1", "1", "top", "leaf", "sp", "1"]);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn l1_tar_unsafe_path_rejected() {
    // Build malicious tars with hand-crafted raw ustar headers: the `tar`
    // crate's safe Builder itself rejects `..`/absolute paths at append
    // time, so raw headers are the only way to test the extractor guard.
    fn raw_tar_one(path: &std::path::Path, name: &str, typeflag: u8, linkname: &str, data: &[u8]) {
        let mut hdr = [0u8; 512];
        let bytes = name.as_bytes();
        assert!(bytes.len() < 100, "test name fits");
        hdr[..bytes.len()].copy_from_slice(bytes);
        hdr[100..108].copy_from_slice(b"0000644\0");
        hdr[108..116].copy_from_slice(b"0000000\0");
        hdr[116..124].copy_from_slice(b"0000000\0");
        let size_oct = format!("{:011o}\0", data.len());
        hdr[124..136].copy_from_slice(size_oct.as_bytes());
        hdr[136..148].copy_from_slice(b"00000000000\0");
        hdr[148..156].copy_from_slice(b"        ");
        hdr[156] = typeflag;
        hdr[257..262].copy_from_slice(b"ustar");
        hdr[262] = 0;
        hdr[263..265].copy_from_slice(b"00");
        if !linkname.is_empty() {
            let lb = linkname.as_bytes();
            hdr[157..157 + lb.len()].copy_from_slice(lb);
        }
        let sum: u32 = hdr.iter().map(|&b| b as u32).sum();
        let chk = format!("{sum:06o}\0 ");
        hdr[148..156].copy_from_slice(chk.as_bytes());
        let mut tar = Vec::new();
        tar.extend_from_slice(&hdr);
        tar.extend_from_slice(data);
        while tar.len() % 512 != 0 {
            tar.push(0);
        }
        tar.extend_from_slice(&[0u8; 1024]);
        std::fs::write(path, &tar).expect("write raw tar");
    }
    let dir = mktemp_dir("evil");
    let d = dir.to_string_lossy().into_owned();
    // 1. Parent-dir entry.
    raw_tar_one(&dir.join("evil.tar"), "../evil.txt", b'0', "", b"evil");
    let src = format!("fn main() -> i32 {{ print(tar_extract(\"{d}/evil.tar\", \"{d}/out\")) return 0 }}");
    let err = run_err(&src);
    assert_eq!(err.code, "E-COMPRESS-UNSAFE-PATH", "{}", err.to_json());
    // 2. Absolute-path entry.
    raw_tar_one(
        &dir.join("abs.tar"),
        "/tmp/l1_abs_evil.txt",
        b'0',
        "",
        b"abs",
    );
    let src2 = format!("fn main() -> i32 {{ print(tar_extract(\"{d}/abs.tar\", \"{d}/out2\")) return 0 }}");
    let err2 = run_err(&src2);
    assert_eq!(err2.code, "E-COMPRESS-UNSAFE-PATH", "{}", err2.to_json());
    // 3. Symlink entry.
    raw_tar_one(
        &dir.join("link.tar"),
        "evil-link",
        b'2',
        "/etc/passwd",
        b"",
    );
    let src3 = format!("fn main() -> i32 {{ print(tar_extract(\"{d}/link.tar\", \"{d}/out3\")) return 0 }}");
    let err3 = run_err(&src3);
    assert_eq!(err3.code, "E-COMPRESS-UNSAFE-PATH", "{}", err3.to_json());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn l1_err_io_not_found_for_files() {
    let dir = mktemp_dir("ionotfound");
    let d = dir.to_string_lossy().into_owned();
    for (tmpl, want) in [
        (
            format!("fn main() -> i32 {{ print(gzip_file(\"{d}/nope.txt\", \"{d}/o.gz\")) return 0 }}"),
            "E-IO-NOT-FOUND",
        ),
        (
            format!("fn main() -> i32 {{ print(gunzip_file(\"{d}/nope.gz\", \"{d}/o\")) return 0 }}"),
            "E-IO-NOT-FOUND",
        ),
        (
            format!("fn main() -> i32 {{ print(tar_extract(\"{d}/nope.tar\", \"{d}/o\")) return 0 }}"),
            "E-IO-NOT-FOUND",
        ),
        (
            format!("fn main() -> i32 {{ print(tar_create(\"{d}/nope-dir\", \"{d}/o.tar\")) return 0 }}"),
            "E-IO-NOT-FOUND",
        ),
    ] {
        let err = run_err(&tmpl);
        assert_eq!(err.code, want, "{tmpl}: {}", err.to_json());
    }
    std::fs::remove_dir_all(&dir).ok();
}

// ---- arity / static + dynamic types ----

#[test]
fn l1_static_arity_and_types() {
    check_err_code("fn main() -> i32 { print(dt_now(1)) return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { print(dt_parse_iso()) return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { print(dt_format(0)) return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { print(dt_from_parts(2024, 1, 1, 0, 0)) return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { print(gzip_compress()) return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { print(tar_extract(\"a\")) return 0 }", "E-ARITY");
    check_err_code("fn main() -> i32 { print(dt_parse_iso(42)) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { print(dt_format(\"x\", \"%Y\")) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { print(dt_add(0, 1)) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { print(gzip_compress(42)) return 0 }", "E-TYPE");
    check_err_code("fn main() -> i32 { print(tar_create(1, 2)) return 0 }", "E-TYPE");
}

#[test]
fn l1_dynamic_etype_no_panic() {
    let err = run_err("fn main() -> i32 { let m = {\"k\": 1} print(dt_parse_iso(m[\"k\"])) return 0 }");
    assert_eq!(err.code, "E-TYPE", "got: {}", err.to_json());
    let err = run_err("fn main() -> i32 { let m = {\"k\": \"x\"} print(dt_format(m[\"k\"], \"%Y\")) return 0 }");
    assert_eq!(err.code, "E-TYPE", "got: {}", err.to_json());
    let err = run_err("fn main() -> i32 { let m = {\"k\": 1} print(dt_add(0, m[\"k\"])) return 0 }");
    assert_eq!(err.code, "E-TYPE", "got: {}", err.to_json());
    let err = run_err("fn main() -> i32 { let m = {\"k\": 1} print(gzip_compress(m[\"k\"])) return 0 }");
    assert_eq!(err.code, "E-TYPE", "got: {}", err.to_json());
}

// ---- hostile input never panics ----

#[test]
fn l1_hostile_no_panic() {
    let cases = [
        "fn main() -> i32 { print(dt_parse_iso(\"2024-02-29T25:99:99Z\")) return 0 }",
        "fn main() -> i32 { print(dt_format(9223372036854775807, \"%Y\")) return 0 }",
        "fn main() -> i32 { print(dt_format(0, \"%Y%m%d%H%M%S%f%z%a%A%b%B%j%p%%\")) return 42 }",
        "fn main() -> i32 { print(dt_add(0, {})) print(dt_add(0, {\"bogus\": 5})) return 42 }",
        "fn main() -> i32 { print(gzip_decompress(\"\")) return 0 }",
        "fn main() -> i32 { print(zlib_decompress(\"ffffffff\")) return 0 }",
        "fn main() -> i32 { print(dt_from_parts(99999, 99, 99, 99, 99, 99)) return 0 }",
        "fn main() -> i32 { print(dt_parts_offset(0, 0 - 2000)) return 0 }",
    ];
    for src in cases {
        let mut p = Parser::new(src);
        let prog = p.parse_program().expect("parses");
        assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok(), "{src}");
        let mir = klang::mir::lower(&prog);
        let _ = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new());
    }
    let (v, out) = run_ok("fn main() -> i32 { print(dt_format(0, \"%Y%m%d%H%M%S%f%z%a%A%b%B%j%p%%\")) print(dt_add(0, {}) == 0) return 42 }");
    assert_eq!(v, 42);
    assert_eq!(out[1], "1");
}

// ---- sandbox ----

#[test]
fn l1_sandbox_datetime_and_string_forms_allowed() {
    let (v, out) = run_sandboxed("fn main() -> i32 { print(dt_now() > 0) print(dt_parse_iso(\"2024-01-01\") == 1704067200000) print(gzip_decompress(gzip_compress(\"sandbox-ok\")) == \"sandbox-ok\") print(zlib_decompress(zlib_compress(\"z\")) == \"z\") return 42 }")
        .expect("datetime+string compress allowed in sandbox");
    assert_eq!(v, 42, "out: {out:?}");
    assert_eq!(out, vec!["1", "1", "1", "1"]);
}

#[test]
fn l1_sandbox_file_forms_follow_temp_rule() {
    // Inside the temp dir: allowed.
    let dir = mktemp_dir("sbok");
    let d = dir.to_string_lossy().into_owned();
    std::fs::write(dir.join("in.txt"), b"hi").expect("write");
    // NOTE: run_sandboxed locks internally; do not hold L1_LOCK here.
    {
        let _guard = L1_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        klang::sandbox::set_sandbox(true);
        let src = format!("fn main() -> i32 {{ print(gzip_file(\"{d}/in.txt\", \"{d}/in.gz\")) return 42 }}");
        let mut p = Parser::new(&src);
        let prog = p.parse_program().expect("parses");
        assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok());
        let mir = klang::mir::lower(&prog);
        let r = klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new());
        klang::sandbox::set_sandbox(false);
        assert!(r.is_ok(), "temp-dir file op allowed: {r:?}");
    }
    // Outside the temp dir: denied with E-SANDBOX-FILE.
    for src in [
        "fn main() -> i32 { print(gzip_file(\"/etc/hostname\", \"/tmp/l1_sb_out.gz\")) return 0 }",
        "fn main() -> i32 { print(gzip_file(\"../outside.txt\", \"out.gz\")) return 0 }",
        "fn main() -> i32 { print(tar_extract(\"/etc/hostname\", \"/tmp/l1_sb_tar\")) return 0 }",
    ] {
        let err = run_sandboxed(src).expect_err("sandbox must deny");
        assert!(err.starts_with("E-SANDBOX-FILE"), "{src}: got {err}");
    }
    std::fs::remove_dir_all(&dir).ok();
}

// ---- backend parity: JIT and v2 reject loudly ----

#[test]
fn l1_jit_rejects_new_builtins() {
    for src in [
        "fn main() -> i32 { print(dt_now()) return 0 }",
        "fn main() -> i32 { print(gzip_compress(\"x\")) return 0 }",
        "fn main() -> i32 { print(tar_create(\"a\", \"b\")) return 0 }",
        "fn main() -> i32 { print(dt_is_leap(2024)) return 0 }",
    ] {
        let mut p = Parser::new(src);
        let prog = p.parse_program().expect("parses");
        assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok(), "{src}");
        let mir = klang::mir::lower(&prog);
        let err = klang::jit::run_jit(&mir, "main", &[]).expect_err("JIT must reject");
        assert!(err.contains("unsupported"), "loud JIT-FAIL, got: {err}");
    }
}

#[test]
fn l1_v2_rejects_new_builtins() {
    let src = "fn main() -> i32 {\n    print(dt_now())\n    return 0\n}\n";
    let prog = klang::parser::v2::parse_v2_program(src).expect("v2 parses");
    let err = klang::runtime::v2::run_v2_program(&prog, "main").expect_err("v2 must reject");
    assert_eq!(err.code, "E-UNDEFINED", "got: {}", err.to_json());
}

// ---- logship example ----

#[test]
fn l1_logship_example_runs_and_fmt_clean() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let prog_path = root.join("examples/apps/logship/main.klang");
    assert!(prog_path.is_file(), "logship example exists");
    let src = std::fs::read_to_string(&prog_path).expect("read example");
    // fmt must stay clean for new .klang files.
    let fmt = std::process::Command::new(klang_bin())
        .args(["fmt", "--check", prog_path.to_str().unwrap()])
        .output()
        .expect("fmt runs");
    assert!(
        fmt.status.success(),
        "fmt --check must pass: {}",
        String::from_utf8_lossy(&fmt.stderr)
    );
    let _guard = L1_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    klang::sandbox::set_sandbox(false);
    let mut p = Parser::new(&src);
    let prog = p.parse_program().expect("example parses");
    assert!(klang::hir::TypedHIR::check(prog.clone()).is_ok(), "example checks");
    let mir = klang::mir::lower(&prog);
    let (v, out) =
        klang::runtime::run_with_output(&mir, "main", &[], &HashMap::new()).expect("example runs");
    klang::sandbox::set_sandbox(false);
    assert_eq!(v, 0, "out: {out:?}");
    let joined = out.join("\n");
    assert!(joined.contains("2024-02-28"), "day 1: {out:?}");
    assert!(joined.contains("2024-02-29"), "leap day: {out:?}");
    assert!(joined.contains("2024-03-01"), "day 3: {out:?}");
}
