//! 停止 on a long wait (生成, 軟規則, DOI) aborts the browser's fetch: the
//! connection closes, and the server drops the handler's future with it —
//! a Claude Code child (`kill_on_drop`) is killed, a request to the local
//! model or Crossref is cancelled. This pins that behaviour of the HTTP
//! stack the server is built on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct Flag(Arc<AtomicBool>);
impl Drop for Flag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn a_closed_connection_drops_the_handler() {
    let dropped = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let (d, f) = (dropped.clone(), finished.clone());
    let app = axum::Router::new().route(
        "/slow",
        axum::routing::post(move || {
            let (d, f) = (d.clone(), f.clone());
            async move {
                let _guard = Flag(d);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                f.store(true, Ordering::SeqCst);
                "done"
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let req = tokio::spawn(async move { reqwest::Client::new().post(format!("http://{addr}/slow")).body("{}").send().await });
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    req.abort(); // what AbortController does to the browser's connection
    for _ in 0..40 {
        if dropped.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(dropped.load(Ordering::SeqCst), "the handler is dropped soon after the client goes");
    assert!(!finished.load(Ordering::SeqCst), "it does not run to the end");
}
