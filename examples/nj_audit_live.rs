use hitung_midterm::{config::Reference,feed,model::FeedResult,render,projection::ProjResult};
#[tokio::main]
async fn main() {
 let reference=Reference::load();
 let cfg=reference.sources.iter().find(|s|s.state=="NJ").unwrap();
 let client=reqwest::Client::builder().user_agent("hitung-midterm/0.1 (+https://github.com/hyderoo/hitung-midterm)").timeout(std::time::Duration::from_secs(40)).build().unwrap();
 let first=client.get(&cfg.export_url).send().await.unwrap().bytes().await.unwrap();
 let result=feed::parse_njclarity(&client,&first,cfg).await.unwrap();
 std::fs::write("/tmp/nj-audited-live.json",serde_json::to_string_pretty(&result).unwrap()).unwrap();
 std::fs::write("/tmp/nj-audited-preview.html",render::page(&reference,&[FeedResult::Ok(result)],&ProjResult::Failed("off".into()),render::View::House,chrono::Utc::now())).unwrap();
}
