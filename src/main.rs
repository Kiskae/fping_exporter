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

use std::{
    collections::HashMap, convert::Infallible, env, io, marker::PhantomData, time::Duration,
};

use clap::crate_version;
use prometheus::{labels, opts};
use semver::VersionReq;
use tokio::sync::oneshot;

mod args;
mod event_stream;
mod fping;
mod http;
mod util;

use crate::util::{
    lock::{Claim, LockControl},
    signal::{ControlToInterrupt, Interrupted},
};

#[cfg(all(feature = "docker", unix))]
async fn terminate_signal() -> Option<&'static str> {
    // Docker signals container shutdown through SIGTERM
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).ok()?;
    tokio::select! {
        Some(_) = term.recv() => Some("SIGTERM"),
        Ok(_) = tokio::signal::ctrl_c() => Some("SIGINT"),
        else => None
    }
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
struct MetricsState<T, P> {
    last_result: HashMap<String, f64>,
    expected_targets: u32,
    current_targets: u32,
    held_token: Option<T>,
    _marker: PhantomData<P>,
}

impl<T, P> MetricsState<T, P> {
    fn new() -> Self {
        Self {
            last_result: HashMap::default(),
            expected_targets: 1,
            current_targets: 0,
            held_token: None,
            _marker: PhantomData,
        }
    }

    fn calc_ipdv(&mut self, target: &str, rtt: Duration) -> Option<f64> {
        let one_way_delay = rtt.div_f64(2.0).as_secs_f64();
        match self.last_result.get_mut(target) {
            Some(prev) => {
                let delta = (*prev - one_way_delay).abs();
                *prev = one_way_delay;
                Some(delta)
            }
            None => {
                self.last_result.insert(target.to_owned(), one_way_delay);
                None
            }
        }
    }
}

trait OnSummaryComplete {
    fn on_completed(self);
}

// Either signals are completely disabled
impl OnSummaryComplete for Infallible {
    fn on_completed(self) {}
}

// Or we have exclusive access that has then been successfully applied as
// an interrupt.
impl OnSummaryComplete for Interrupted<(oneshot::Sender<Claim>, Claim)> {
    fn on_completed(self) {
        // The receiver might be gone, this is fine
        let Interrupted((tx, claim)) = self;
        let _ = tx.send(claim);
    }
}

impl<O: AsRef<str>, E: AsRef<str>, H, T: OnSummaryComplete> event_stream::EventHandler
    for MetricsState<T, (O, E, H)>
{
    type Output = O;
    type Error = E;
    type Handle = H;
    type Token = T;

    fn on_output(&mut self, event: Self::Output) {
        if let Some(ping) = fping::Ping::parse(&event) {
            if let Some(rtt) = ping.result {
                let delta = self.calc_ipdv(ping.target, rtt);

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

    fn on_error(&mut self, event: Self::Error) {
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
                    let token = self.held_token.take().expect("missing token");
                    token.on_completed();
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

    fn on_control(&mut self, _: &mut Self::Handle, token: Self::Token) -> io::Result<()> {
        self.held_token = Some(token);
        Ok(())
    }
}

fn info_metric(ver: semver::Version) -> Box<dyn prometheus::core::Collector> {
    let ver = ver.to_string();
    let metric = prometheus::Counter::with_opts(opts!(
        "fping_info",
        "exporter runtime information",
        labels! {
            "version" => crate_version!(),
            "fping_version" => &ver
        }
    ))
    .unwrap();
    metric.inc();
    Box::new(metric)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();
    let fping_binary = env::var("FPING_BIN").unwrap_or_else(|_| "fping".into());
    let launcher = fping::for_program(&fping_binary);
    let args = args::load_args(&launcher, discovery_timeout()).await?;

    prometheus::register(info_metric(args.fping_version.clone()))?;

    let (http_tx, rx) = if VersionReq::parse(">=4.3.0")
        .unwrap()
        .matches(&args.fping_version)
    {
        info!("SIGQUIT signal summary enabled");
        http::RegistryAccess::new(prometheus::default_registry(), Some(1))
    } else {
        warn!(
            "fping {} does not support summary requests, accurate packet loss will not be available",
            args.fping_version
        );
        http::RegistryAccess::new(prometheus::default_registry(), None)
    };

    let mut fping = launcher.spawn(&args.targets).await?.with_controls(rx);

    tokio::select! {
        e = terminate_signal() => {
            match e {
                Some(signal) => debug!("received {}", signal),
                None => error!("failure registering signal handler")
            }
        },
        res = fping.listen(LockControl::new(ControlToInterrupt::new(MetricsState::new(), nix::sys::signal::SIGQUIT))) => {
            // fping should be
            error!("fping listener terminated:\n{:#?}", res);
            res?;
        },
        res = http::publish_metrics(&args.metrics, http_tx) => {
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
        None => {
            use crate::util::signal::Interruptable;

            // Send SIGINT and clean up
            handle.interrupt(nix::sys::signal::SIGINT)?;
            handle.wait().await?;
        }
    }

    Ok(())
}
