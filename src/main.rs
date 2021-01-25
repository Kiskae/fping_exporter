#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
#[macro_use]
extern crate clap;

use std::{convert::Infallible, env, time::Duration};

use cfg_if::cfg_if;
use semver::VersionReq;

mod args;
mod event_stream;
mod fping;

async fn terminate_signal() -> Option<()> {
    cfg_if! {
        if #[cfg(all(feature = "docker", unix))] {
            // Docker signals container shutdown through SIGTERM
            use tokio::signal::unix::{signal, SignalKind};
            signal(SignalKind::terminate()).ok()?.recv().await
        } else {
            tokio::signal::ctrl_c().await.ok()
        }
    }
}

async fn metrics_handler(
    args: &args::MetricArgs,
    // registry, interrupt channel, perhaps combined?
    // shutdown can perhaps be derived from args
) -> Result<(), warp::Error> {
    use warp::Filter;

    let handler = || async {
        //TODO: request summary update
        //TODO: emit registry output
        Ok::<_, Infallible>("well done!")
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();

    let fping_binary = env::var("FPING_BIN").unwrap_or_else(|_| "fping".into());
    let launcher = fping::for_program(&fping_binary);
    let args = args::load_args(launcher.version(Duration::from_millis(5000)).await)?;

    if VersionReq::parse(">=4.3.0")
        .unwrap()
        .matches(&args.fping_version)
    {
        info!("supports signal summary");
    } else {
        warn!("fping {} does not support summary requests, packet loss may be inaccurate", args.fping_version);
    }

    // change behavior based on args.fping_version
    let mut fping = launcher.spawn(&args.targets).await?;

    tokio::select! {
        //TODO: terminate_signal => None -> failure to register handler
        Some(_) = terminate_signal() => {
            info!("received term")
            //TODO: log terminate signal
        },
        res = fping.listen(|ev| trace!(target: "fping", "{:?}", ev)) => {
            res?;
            // Unexpected end, fall through and let clean up handle it
        },
        res = metrics_handler(&args.metrics) => {
            res?;
            error!("execution timeout")
            //TODO: log execution timeout
        }
    }

    // Clean up fping
    let mut handle = fping.inner();
    match handle.try_wait()? {
        //TODO: try to diagnose based on status
        //TODO: check for unhandled stderr output for reason?
        Some(status) => error!("{:?}", status),
        // Exit not caused by unexpected fping exit, clean up the child process
        //TODO: fping uses SIGINT as kill signal, .kill() defaults to SIGKILL
        None => handle.kill().await?,
    }

    Ok(())
}
