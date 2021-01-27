use clap::Arg;
use std::{
    net::{AddrParseError, SocketAddr},
    num::ParseIntError,
    time::Duration,
};
use thiserror::Error;

use crate::fping::{version::VersionError, Launcher};

#[derive(Debug, Error)]
pub enum ArgsError {
    #[error("metrics-port is a not a valid port: {0}")]
    PortNotANumber(#[from] ParseIntError),
    #[error("metrics-bind is not a valid ip: {0}")]
    MalformedBind(#[from] AddrParseError),
    #[error(transparent)]
    FpingProblem(#[from] VersionError),
    #[error("runtime-limit is not valid duration: {0}")]
    NotAValidTimeout(#[from] humantime::DurationError),
    #[error(transparent)]
    #[cfg(test)]
    TestError(#[from] clap::Error),
}

#[derive(Debug)]
pub struct MetricArgs {
    pub addr: SocketAddr,
    pub path: String,
    pub runtime_limit: Option<Duration>,
}

#[derive(Debug)]
pub struct Args {
    pub fping_version: semver::Version,
    pub metrics: MetricArgs,
    pub targets: Vec<String>,
}

fn format_long_version(fping: Option<&semver::Version>) -> String {
    format!(
        "v{}\nfping: {}",
        crate_version!(),
        fping.map_or_else(|| "<not found>".to_string(), |x| x.to_string())
    )
}

fn clap_app() -> clap::App<'static, 'static> {
    app_from_crate!()
        .arg(
            Arg::with_name("path")
                .takes_value(true)
                .long("metrics-path")
                .default_value("metrics"),
        )
        .arg(
            Arg::with_name("port")
                .takes_value(true)
                .long("metrics-port")
                .default_value("9775"),
        )
        .arg(
            Arg::with_name("bind")
                .takes_value(true)
                .long("metrics-bind")
                .default_value("::"),
        )
        .arg(
            Arg::with_name("timeout")
                .takes_value(true)
                .long("runtime-limit"),
        )
        .arg(
            Arg::with_name("TARGET")
                .required(true)
                .multiple(true)
                .help("hostname or ip address to ping"),
        )
}

fn convert_to_args(
    args: clap::ArgMatches,
    fping_version: semver::Version,
) -> Result<Args, ArgsError> {
    //FIXME: target specification through files?
    let targets = args
        .values_of("TARGET")
        .map_or_else(Vec::new, |iter| iter.map(|s| s.to_owned()).collect());

    let runtime_limit = args
        .value_of("timeout")
        .map(humantime::parse_duration)
        .transpose()?;

    Ok(Args {
        fping_version,
        metrics: MetricArgs {
            addr: SocketAddr::new(
                args.value_of("bind").unwrap().parse()?,
                args.value_of("port").unwrap().parse()?,
            ),
            path: args.value_of("path").unwrap().to_owned(),
            runtime_limit,
        },
        targets,
    })
}

pub async fn load_args(
    launcher: &Launcher<'_>,
    discover_timeout: Duration,
) -> Result<Args, ArgsError> {
    let version = launcher.version(discover_timeout).await;
    convert_to_args(
        clap_app()
            .long_version(format_long_version(version.as_ref().ok()).as_str())
            .get_matches(),
        version?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_cmd(mut args: Vec<&str>) -> Result<Args, ArgsError> {
        args.insert(0, "program_path");
        let matches = clap_app().get_matches_from_safe(args)?;
        convert_to_args(matches, semver::Version::new(1, 0, 0))
    }

    #[test]
    fn basic_usage() {
        parse_cmd(vec!["dns.google"]).unwrap();
    }
}
