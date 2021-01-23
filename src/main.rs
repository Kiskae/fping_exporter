#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate clap;

use std::{convert::Infallible, env};

use cfg_if::cfg_if;
use semver::VersionReq;

mod args;
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

async fn fping_run() -> Option<()> {
    std::future::pending().await
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
    let fping_binary = env::var("FPING_BIN").unwrap_or_else(|_| "fping".into());
    let launcher = fping::for_program(&fping_binary);
    let args = args::load_args(&launcher).await?;
    println!("{:#?}", args);

    if VersionReq::parse(">=4.3.0")
        .unwrap()
        .matches(&args.fping_version)
    {
        println!("supports signal summary");
    }
    // change behavior based on args.fping_version
    //TODO: launch fping process
    //multiplex metrics/fping/cancellation
    let _fping = launcher.spawn().await;

    tokio::select! {
        //TODO: terminate_signal => None -> failure to register handler
        Some(_) = terminate_signal() => {
            println!("received term")
            //TODO: log terminate signal
        },
        _ = fping_run() => {
            println!("fping exit")
        },
        res = metrics_handler(&args.metrics) => {
            res?;
            println!("execution timeout")
            //TODO: log execution timeout
        }
    }

    // Clean up fping

    Ok(())
}
