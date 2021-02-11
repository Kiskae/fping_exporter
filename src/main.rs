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
    collections::HashMap,
    convert::Infallible,
    env, io,
    marker::PhantomData,
    sync::{Arc, Mutex},
    time::Duration,
};

use clap::crate_version;
use prom::{LockedCollector, PingMetrics};
use prometheus::{labels, opts};
use semver::VersionReq;
use tokio::sync::oneshot;

mod args;
mod event_stream;
mod fping;
mod prom;
mod util;

use crate::util::{
    lock::{Claim, LockControl},
    signal::{ControlToInterrupt, Interruptable, Interrupted, KnownSignals},
    NoPrelaunchControl,
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
    metrics: Arc<Mutex<PingMetrics>>,
    _marker: PhantomData<P>,
}

impl<T, P> MetricsState<T, P> {
    fn new(metrics: Arc<Mutex<PingMetrics>>) -> Self {
        Self {
            last_result: HashMap::default(),
            expected_targets: 1,
            current_targets: 0,
            held_token: None,
            metrics,
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

    fn is_alive(&self) -> bool;
}

// Either signals are completely disabled
impl OnSummaryComplete for Infallible {
    fn on_completed(self) {}

    fn is_alive(&self) -> bool {
        false
    }
}

// Or we have exclusive access that has then been successfully applied as
// an interrupt.
impl OnSummaryComplete for Interrupted<(oneshot::Sender<Claim>, Claim)> {
    fn on_completed(self) {
        // The receiver might be gone, this is fine
        let Interrupted((tx, claim)) = self;
        let _ = tx.send(claim);
    }

    fn is_alive(&self) -> bool {
        let Interrupted((ref tx, _)) = self;
        !tx.is_closed()
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
            let labels = ping.labels();
            let delta = if let Some(rtt) = ping.result {
                let delta = self.calc_ipdv(ping.target, rtt);

                trace!("rtt {:?} on {:?}", ping.result, labels);
                trace!("ipvd {:?} on {:?}", delta, labels);
                delta
            } else {
                trace!("timeout on {:?}", labels);
                None
            };
            self.metrics.lock().unwrap().ping(ping, delta);
        } else {
            error!("unhandled stdout: {}", event.as_ref());
        }

        if let Some(token) = self.held_token.as_ref() {
            if !token.is_alive() {
                debug!("dropping dead token");
                self.held_token = None;
            }
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
                self.metrics.lock().unwrap().summary(summary);
                self.current_targets += 1;
                trace!(
                    "{} out of {} targets summarized",
                    self.current_targets,
                    self.expected_targets
                );
                if self.current_targets == self.expected_targets {
                    if let Some(token) = self.held_token.take() {
                        token.on_completed();
                    } else {
                        warn!("summary received, but no token held")
                    }
                }
            }
            Control::SummaryLocalTime => {
                if self.held_token.is_none() {
                    warn!("summary manually triggered, may race with metrics output");
                }

                // Reset expected targets
                self.expected_targets = std::cmp::max(self.expected_targets, self.current_targets);
                self.current_targets = 0;
            }
            Control::Unhandled(err) => {
                debug!("unexpected stderr:\n{}", err);
            }
            e => {
                trace!("ignored output: {:?}", e);
                self.metrics.lock().unwrap().error(e);
            }
        }
    }

    fn on_control(&mut self, _: &mut Self::Handle, token: Self::Token) -> io::Result<()> {
        trace!("control token received");
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

    let metrics = prom::PingMetrics::new("fping");
    prometheus::register(Box::new(LockedCollector::from(metrics.clone())))?;
    prometheus::register(info_metric(args.fping_version.clone()))?;

    let (http_tx, rx) = if VersionReq::parse(">=4.3.0")
        .unwrap()
        .matches(&args.fping_version)
    {
        info!("SIGQUIT signal summary enabled");
        prom::RegistryAccess::new(prometheus::default_registry(), Some(1))
    } else {
        warn!(
            "fping {} does not support summary requests, accurate packet loss will not be available",
            args.fping_version
        );
        prom::RegistryAccess::new(prometheus::default_registry(), None)
    };

    let mut fping = launcher.spawn(&args.targets).await?.with_controls(rx);

    tokio::select! {
        e = terminate_signal() => {
            match e {
                Some(signal) => debug!("received {}", signal),
                None => error!("failure registering signal handler")
            }
        },
        res = fping.listen(NoPrelaunchControl::new(
            LockControl::new(
                ControlToInterrupt::new(
                    MetricsState::new(metrics),
                    KnownSignals::sigquit()
                )
            )
        )) => {
            // fping should be in a permanent loop
            error!("fping listener terminated:\n{:#?}", res);
            res?;
        },
        res = prom::publish_metrics(&args.metrics, http_tx) => {
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
        None => {
            // Send SIGINT and clean up
            handle.interrupt(KnownSignals::sigint())?;
            handle.wait().await?;
        }
    }

    Ok(())
}
