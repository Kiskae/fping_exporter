#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
#[macro_use]
extern crate clap;

use std::{convert::Infallible, env, io, time::Duration};

use semver::VersionReq;

mod args;
mod event_stream;
mod fping;

#[cfg(all(feature = "docker", unix))]
async fn terminate_signal() -> Option<()> {
    // Docker signals container shutdown through SIGTERM
    use tokio::signal::unix::{signal, SignalKind};
    signal(SignalKind::terminate()).ok()?.recv().await
}

#[cfg(not(all(feature = "docker", unix)))]
async fn terminate_signal() -> Option<()> {
    tokio::signal::ctrl_c().await.ok()
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

#[cfg(debug_assertions)]
fn discovery_timeout() -> Duration {
    humantime::parse_duration(option_env!("DEV_PROGRAM_TIMEOUT").unwrap_or("50ms"))
        .expect("invalid program timeout provided")
}

#[cfg(not(debug_assertions))]
fn discovery_timeout() -> Duration {
    // 50ms to execute a static binary should be plenty...
    Duration::from_millis(50)
}

struct TestType;

impl<H, T: std::fmt::Debug> event_stream::EventHandler<String, String, H, T> for TestType {
    fn on_output(&mut self, event: String) {
        trace!(target: "ev", "{:?}", fping::Ping::parse(&event))
    }

    fn on_error(&mut self, event: String) {
        trace!(target: "ev", "{:?}", fping::Control::parse(&event))
    }

    fn on_control(&mut self, _: &mut H, token: T) -> io::Result<()> {
        trace!(target: "ev", "{:?}", token);
        Ok(())
    }
}

#[allow(dead_code)]
#[derive(Debug)]
struct ExclusiveClaim<T> {
    value: T,
    guard: tokio::sync::OwnedMutexGuard<()>,
}

struct LockControl<H> {
    handler: H,
    lock: std::sync::Arc<tokio::sync::Mutex<()>>,
}

impl<H> LockControl<H> {
    fn wrap(handler: H) -> Self {
        Self {
            handler,
            lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

impl<F, O, E, H, T> event_stream::EventHandler<O, E, H, T> for LockControl<F>
where
    F: event_stream::EventHandler<O, E, H, ExclusiveClaim<T>>,
{
    fn on_output(&mut self, event: O) {
        self.handler.on_output(event)
    }

    fn on_error(&mut self, event: E) {
        self.handler.on_error(event)
    }

    fn on_control(&mut self, handle: &mut H, token: T) -> io::Result<()> {
        if let Ok(claim) = self.lock.clone().try_lock_owned() {
            self.handler.on_control(
                handle,
                ExclusiveClaim {
                    value: token,
                    guard: claim,
                },
            )
        } else {
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();
    let fping_binary = env::var("FPING_BIN").unwrap_or_else(|_| "fping".into());
    let launcher = fping::for_program(&fping_binary);
    let args = args::load_args(launcher.version(discovery_timeout()).await)?;

    if VersionReq::parse(">=4.3.0")
        .unwrap()
        .matches(&args.fping_version)
    {
        info!("supports signal summary");
    } else {
        warn!(
            "fping {} does not support summary requests, packet loss may be inaccurate",
            args.fping_version
        );
    }

    // change behavior based on args.fping_version
    let mut fping = launcher.spawn(&args.targets).await?; //.with_controls(None);

    tokio::select! {
        //TODO: terminate_signal => None -> failure to register handler
        Some(_) = terminate_signal() => {
            info!("received term")
            //TODO: log terminate signal
        },
        res = fping.listen(LockControl::wrap(TestType)) => {
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
    let mut handle = fping.dispose();
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
