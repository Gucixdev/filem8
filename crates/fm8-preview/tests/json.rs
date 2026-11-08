use fm8_preview::json::*;
#[test]
fn parses_and_accesses() {
    let v = parse(r#" {"a":[1,2.5,-3e2,true,null],"s":"x\n\u00e9\ud83d\ude00","o":{"k":"v"}} "#.as_bytes()).unwrap();
    assert_eq!(v["a"].as_array().unwrap().len(), 5);
    assert_eq!(v.pointer("/a/0").unwrap().as_i64(), Some(1));
    assert_eq!(v["a"].as_array().unwrap()[2].as_f64(), Some(-300.0));
    assert_eq!(v["s"].as_str(), Some("x\né😀"));
    assert_eq!(v.pointer("/o/k").unwrap().as_str(), Some("v"));
    assert!(v["missing"] == Value::Null);
}
#[test]
fn rejects_garbage() {
    for s in ["", "{", "[1,]", "{\"a\" 1}", "tru", "\"a", "1 2", "[\"\u{1}\"]"] { assert!(parse(s.as_bytes()).is_none(), "{s:?}"); }
}
