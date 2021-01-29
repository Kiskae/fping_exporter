use std::convert::Infallible;

use prometheus::{proto::MetricFamily, Encoder, Registry, TextEncoder};
use tokio::sync::{mpsc, oneshot};
use warp::{reply::with_header, Rejection, Reply};

use crate::args::MetricArgs;

fn encode_metrics<E: Encoder + Default>(metrics: &[MetricFamily]) -> prometheus::Result<impl Reply> {
    let enc: E = Default::default();
    let mut out = Vec::new();
    enc.encode(metrics, &mut out)?;
    Ok(with_header(out, "Content-Type", enc.format_type()))
}

#[derive(Debug)]
pub enum RegistryAccess<T = Infallible> {
    Limited(Registry, mpsc::Sender<oneshot::Sender<T>>),
    Unlimited(Registry),
}

#[derive(Debug, thiserror::Error)]
enum AccessError {
    #[error("fping process terminated")]
    FpingProcessDead,
    #[error("another request still in progress")]
    RequestDropped(#[from] oneshot::error::RecvError),
}

impl warp::reject::Reject for AccessError {}

impl<T> RegistryAccess<T> {
    pub fn new(
        reg: &Registry,
        buffer: Option<usize>,
    ) -> (Self, Option<mpsc::Receiver<oneshot::Sender<T>>>) {
        match buffer {
            Some(buffer) => {
                let (tx, rx) = mpsc::channel(buffer);
                (Self::Limited(reg.clone(), tx), Some(rx))
            }
            None => (Self::Unlimited(reg.clone()), None),
        }
    }

    async fn gather(self) -> Result<Vec<MetricFamily>, AccessError> {
        match self {
            RegistryAccess::Limited(reg, tx) => {
                let (tx2, rx) = oneshot::channel();
                tx.send(tx2)
                    .await
                    .map_err(|_| AccessError::FpingProcessDead)?;
                // guard using return value
                let _ = rx.await?;
                Ok(reg.gather())
            }
            RegistryAccess::Unlimited(reg) => Ok(reg.gather()),
        }
    }
}

impl<T> Clone for RegistryAccess<T> {
    fn clone(&self) -> Self {
        match *self {
            RegistryAccess::Limited(ref r, ref rx) => {
                RegistryAccess::Limited(r.clone(), rx.clone())
            }
            RegistryAccess::Unlimited(ref r) => RegistryAccess::Unlimited(r.clone()),
        }
    }
}

pub async fn publish_metrics<T: Send + 'static>(
    args: &MetricArgs,
    reg: RegistryAccess<T>,
) -> Result<(), warp::Error> {
    use warp::Filter;

    let handler = move || {
        let reg = reg.clone();
        async move {
            let metrics = reg.gather().await?;

            Ok::<_, Rejection>(encode_metrics::<TextEncoder>(&metrics).unwrap())
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
