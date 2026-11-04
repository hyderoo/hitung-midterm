use hitung_midterm::{config::Reference, load_all, model::FeedResult, render};
use vercel_runtime::{run, service_fn, Error, Request, Response, ResponseBody};

#[tokio::main]
async fn main() -> Result<(), Error> {
    run(service_fn(handler)).await
}

async fn handler(req: Request) -> Result<Response<ResponseBody>, Error> {
    let query = req.uri().query().unwrap_or("");
    let param = |k: &str| {
        query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(key, _)| *key == k)
            .map(|(_, v)| v.to_string())
    };
    // Only these query forms are served. Anything else is redirected to a
    // canonical URL without touching the sources, so random query strings
    // cannot bypass the edge cache and trigger large refetches.
    let canonical = match (query, param("view").as_deref(), param("format").as_deref()) {
        ("", _, _) => None,
        ("view=senat", _, _) | ("view=dpr", _, _) | ("format=json", _, _) => None,
        (_, Some("dpr"), _) => Some("/?view=dpr"),
        _ => Some("/"),
    };
    if let Some(to) = canonical {
        return Ok(Response::builder()
            .status(308)
            .header("Location", to)
            .header("Cache-Control", "public, max-age=0, s-maxage=3600")
            .body(ResponseBody::from(String::new()))?);
    }
    let reference = Reference::load();
    let (feeds, proj) = load_all(&reference).await;
    let now = chrono::Utc::now();
    let any_failed = feeds.iter().any(|f| matches!(f, FeedResult::Failed { .. })) || matches!(proj, hitung_midterm::projection::ProjResult::Failed(_));
    // Edge cache: the CDN refetches the sources at most about every 2 minutes
    // and may serve a stale copy while it revalidates. Failures are cached briefly.
    let cache = if any_failed {
        "public, max-age=0, s-maxage=20"
    } else {
        "public, max-age=0, s-maxage=120, stale-while-revalidate=300"
    };
    let (ctype, body) = if param("format").as_deref() == Some("json") {
        ("application/json; charset=utf-8", render::json(&reference, &feeds, &proj, now))
    } else {
        let view = match param("view").as_deref() {
            Some("dpr") => render::View::House,
            _ => render::View::Senate,
        };
        ("text/html; charset=utf-8", render::page(&reference, &feeds, &proj, view, now))
    };
    Ok(Response::builder()
        .status(200)
        .header("Content-Type", ctype)
        .header("Cache-Control", cache)
        .body(ResponseBody::from(body))?)
}
