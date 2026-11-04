pub mod config;
pub mod feed;
pub mod model;
pub mod projection;
pub mod render;

use config::Reference;
use model::FeedResult;

/// Fetches every configured source concurrently. A failing source never
/// takes the others down: it comes back as FeedResult::Failed.
pub async fn load_all(reference: &Reference) -> (Vec<FeedResult>, projection::ProjResult) {
    let client = reqwest::Client::builder()
        .user_agent("hitung-midterm/0.1 (+https://github.com/hyderoo/hitung-midterm)")
        .timeout(std::time::Duration::from_secs(40))
        .build()
        .expect("http client");
    let names: std::collections::BTreeMap<String, String> =
        reference.map.states.iter().map(|(c, s)| (s.name.clone(), c.clone())).collect();
    let proj_task = {
        let client = client.clone();
        let names = names.clone();
        tokio::spawn(async move {
            if std::env::var("WIKI_UNVERIFIED").ok().as_deref() == Some("1") {
                projection::fetch(&client, &names).await
            } else {
                projection::ProjResult::Failed("off".into())
            }
        })
    };
    let tasks: Vec<_> = reference
        .sources
        .iter()
        .cloned()
        .map(|cfg| {
            let client = client.clone();
            tokio::spawn(async move {
                let r = feed::fetch_state(&client, &cfg).await;
                (cfg, r)
            })
        })
        .collect();
    let mut out = Vec::new();
    for t in tasks {
        match t.await {
            Ok((_, r)) => out.push(r),
            Err(e) => out.push(FeedResult::Failed {
                state: "??".into(),
                source_label: "tugas gagal".into(),
                human_url: String::new(),
                reason: e.to_string(),
            }),
        }
    }
    out.sort_by(|a, b| a.state().cmp(b.state()));
    let proj = proj_task.await.unwrap_or_else(|e| projection::ProjResult::Failed(e.to_string()));
    (out, proj)
}
