#[test]
fn jit_probe_add_returns_42() {
    let v = klang::jit::probe_add().expect("jit probe runs");
    println!("jit probe add(7, 35) = {v}");
    assert_eq!(v, 42);
}
