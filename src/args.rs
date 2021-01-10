use std::{
    net::SocketAddr,
    time::Duration,
};

use crate::fping::{version::VersionError, Launcher};

#[derive(Debug)]
pub struct Args {
    fping_version: semver::Version,
    metrics_addr: SocketAddr,
    metrics_path: String,
}

fn format_long_version(fping: Option<semver::Version>) -> String {
    format!(
        "v{}\nfping: {}",
        crate_version!(),
        fping.map_or_else(|| "<not found>".to_string(), |x| x.to_string())
    )
}

fn clap_app(long_version: &str) -> clap::App {
    //TODO: ARGS
    // metrics-path
    // metrics-port
    // metrics-bind
    // TARGETS

    app_from_crate!().long_version(long_version)
}

fn to_final_args(_args: clap::ArgMatches, fping_version: semver::Version) -> Args {
    Args {
        fping_version,
        metrics_addr: ([0, 0, 0, 0], 9775).into(),
        metrics_path: "metrics".to_owned(),
    }
}

//TODO: create own error, validation of arguments in addition to VersionError
pub async fn load_args(fping: &Launcher<'_>) -> Result<Args, VersionError> {
    let version = fping.version(Duration::from_secs(1)).await;
    Ok(to_final_args(
        clap_app(format_long_version(version.as_ref().ok().cloned()).as_str()).get_matches(),
        version?,
    ))
}
