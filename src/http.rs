use std::convert::Infallible;

use prometheus::{Encoder, Registry, TextEncoder};
use warp::{reply::with_header, Reply};

use crate::args::MetricArgs;

fn encode_metrics<E: Encoder + Default>(reg: &Registry) -> prometheus::Result<impl Reply> {
    let enc: E = Default::default();
    let mut out = Vec::new();
    enc.encode(&reg.gather(), &mut out)?;
    Ok(with_header(out, "Content-Type", enc.format_type()))
}

pub async fn publish_metrics(args: &MetricArgs) -> Result<(), warp::Error> {
    use warp::Filter;

    let handler = move || {
        let reg = prometheus::default_registry();
        async move {
            //TODO: provide guarded access to Registry instead...
            //TODO: request summary update from fping...
            Ok::<_, Infallible>(encode_metrics::<TextEncoder>(&reg).unwrap())
        }
    };

    let metrics = warp::path(args.path.clone())
        .and(warp::path::end())
        .and_then(handler);

    let (_, server) = warp::serve(metrics).try_bind_with_graceful_shutdown(args.addr, {
        info!(target: "metrics", "publishing metrics on http://{}/{}", args.addr, args.path);

        let timeout = args.runtime_limit;
        async move {
            match timeout {
                Some(timeout) => tokio::time::sleep(timeout).await,
                None => std::future::pending().await,
            }
        }
    })?;

    server.await;
    Ok(())
}
