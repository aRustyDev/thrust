// Template: tests/e2e_harness.rs in the harness crate (package `e2e`). Example names: `api` routes, stub UI. [re-run]
#[tokio::test]
async fn e2e_harness_serves_ui_and_api() {
    let app = e2e::TestApp::start().await;
    app.state.items.write().unwrap().insert(1, "seeded".into());

    let index = reqwest::get(format!("{}/", app.base_url)).await.unwrap().text().await.unwrap();
    assert!(index.contains(r#"<div id="main">"#), "{index}");

    let item = reqwest::get(format!("{}/api/items/1", app.base_url)).await.unwrap().text().await.unwrap();
    assert_eq!(item, r#"{"id":1,"name":"seeded"}"#);

    let deep_link = reqwest::get(format!("{}/entries/42", app.base_url)).await.unwrap();
    assert_eq!(deep_link.status().as_u16(), 200, "SPA fallback should serve index.html");
}
