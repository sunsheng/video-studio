fn main() {
    use studio_core::{fixtures, StageId};
    let mut m = serde_json::Map::new();
    for s in StageId::all() {
        m.insert(
            s.as_str().into(),
            serde_json::json!({
                "outputs": fixtures::outputs(s),
                "summary": fixtures::summary(s),
                "confirmation": fixtures::confirmation(s),
            }),
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::Value::Object(m)).unwrap()
    );
}
