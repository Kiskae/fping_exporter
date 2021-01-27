// fping itself only runs on unix
#![cfg(unix)]
// FIXME: remove once testing has been fully covered
#![cfg_attr(test, allow(dead_code))]

#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;
#[macro_use]
extern crate clap;

use std::{collections::HashMap, convert::Infallible, env, io, time::Duration};

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

#[derive(Debug)]
struct MetricsState<T> {
    last_result: HashMap<String, f64>,
    expected_targets: u32,
    current_targets: u32,
    held_token: Option<T>,
}

impl<T> MetricsState<T> {
    fn new() -> Self {
        Self {
            last_result: HashMap::default(),
            expected_targets: 1,
            current_targets: 0,
            held_token: None,
        }
    }
}

impl<O: AsRef<str>, E: AsRef<str>, H, T: std::fmt::Debug> event_stream::EventHandler<O, E, H, T>
    for MetricsState<T>
{
    fn on_output(&mut self, event: O) {
        if let Some(ping) = fping::Ping::parse(&event) {
            let _delta = if let Some(rtt) = ping.result {
                let one_way_delay = rtt.div_f64(2.0).as_secs_f64();
                match self.last_result.get_mut(ping.target) {
                    Some(prev) => {
                        let delta = (*prev - one_way_delay).abs();
                        *prev = one_way_delay;
                        Some(delta)
                    }
                    None => {
                        self.last_result
                            .insert(ping.target.to_owned(), one_way_delay);
                        None
                    }
                }
            } else {
                None
            };
            //TODO: record ping
            debug!("rtt {:?} on [{},{}]", ping.result, ping.target, ping.addr);
            debug!("ipvd {:?} on [{},{}]", _delta, ping.target, ping.addr);
            //TODO: record delta
        }
    }

    fn on_error(&mut self, event: E) {
        use fping::Control;
        match Control::parse(&event) {
            Control::TargetSummary {
                target,
                addr,
                sent,
                received,
            } => {
                debug!("packet loss ({}/{}) on [{},{}]", received, sent, target, addr);
                //TODO: record sent/received
                self.current_targets = self.current_targets + 1;
                if self.current_targets >= self.expected_targets {
                    //TODO: resolve held_token
                    self.held_token = None;
                }
            }
            Control::RandomLocalTime => {
                // Reset expected targets
                self.expected_targets = std::cmp::max(self.expected_targets, self.current_targets);
                self.current_targets = 0;
            }
            e => trace!("Unhandled: {:?}", e),
        }
    }

    fn on_control(&mut self, _: &mut H, token: T) -> io::Result<()> {
        debug_assert!(self.held_token.is_none());
        self.held_token = Some(token);
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();
    let fping_binary = env::var("FPING_BIN").unwrap_or_else(|_| "fping".into());
    let launcher = fping::for_program(&fping_binary);
    let args = args::load_args(&launcher, discovery_timeout()).await?;

    let _ = if VersionReq::parse(">=4.3.0")
        .unwrap()
        .matches(&args.fping_version)
    {
        info!("supports signal summary");
        1
    } else {
        warn!(
            "fping {} does not support summary requests, packet loss may be inaccurate",
            args.fping_version
        );
        0
    };

    // change behavior based on args.fping_version
    let mut fping = launcher
        .spawn(&args.targets)
        .await?
        .with_controls::<u32>(None);

    tokio::select! {
        //TODO: terminate_signal => None -> failure to register handler
        Some(_) = terminate_signal() => {
            info!("received term")
            //TODO: log terminate signal
        },
        res = fping.listen(MetricsState::new()) => {
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
