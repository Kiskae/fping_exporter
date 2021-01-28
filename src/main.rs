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

use std::{collections::HashMap, env, io, time::Duration};

use semver::VersionReq;

mod args;
mod event_stream;
mod fping;
mod http;

#[cfg(all(feature = "docker", unix))]
async fn terminate_signal() -> Option<&'static str> {
    // Docker signals container shutdown through SIGTERM
    use tokio::signal::unix::{signal, SignalKind};
    signal(SignalKind::terminate())
        .ok()?
        .recv()
        .await
        .map(|_| "SIGTERM")
}

#[cfg(not(all(feature = "docker", unix)))]
async fn terminate_signal() -> Option<&'static str> {
    tokio::signal::ctrl_c().await.ok().map(|_| "SIGINT")
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
            if let Some(rtt) = ping.result {
                let one_way_delay = rtt.div_f64(2.0).as_secs_f64();
                let delta = match self.last_result.get_mut(ping.target) {
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
                };

                //TODO: record ping
                trace!("rtt {:?} on {:?}", ping.result, ping.labels());
                //TODO: record delta
                trace!("ipvd {:?} on {:?}", delta, ping.labels());
            } else {
                trace!("timeout on {:?}", ping.labels());
            }
        } else {
            error!("unhandled stdout: {}", event.as_ref());
        }
    }

    fn on_error(&mut self, event: E) {
        use fping::Control;
        match Control::parse(&event) {
            Control::TargetSummary(summary) => {
                trace!(
                    "packet loss ({}/{}) on {:?}",
                    summary.received,
                    summary.sent,
                    summary.labels()
                );
                //TODO: record sent/received
                self.current_targets += 1;
                if self.current_targets == self.expected_targets {
                    let _ = self.held_token.take().expect("missing token");
                    //TODO: resolve held_token
                }
            }
            Control::SummaryLocalTime => {
                // Reset expected targets
                self.expected_targets = std::cmp::max(self.expected_targets, self.current_targets);
                self.current_targets = 0;
            }
            e => trace!("unhandled stderr:\n{:#?}", e),
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
        e = terminate_signal() => {
            match e {
                Some(signal) => debug!("received {}", signal),
                None => error!("failure registering signal handler")
            }
        },
        res = fping.listen(MetricsState::new()) => {
            // fping should be
            error!("fping listener terminated:\n{:#?}", res);
            res?;
        },
        res = http::publish_metrics(&args.metrics) => {
            debug!("http handler terminated:\n{:#?}", res);
            res?;
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
