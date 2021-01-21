use std::{
    io,
    process::{ExitStatus, Output},
};
use thiserror::Error;

use regex::Regex;

fn parse_fping_version(raw: &str) -> Option<semver::Version> {
    lazy_static! {
        static ref VERSION_PATTERN: Regex =
            Regex::new(r"^.+: Version (?P<major>\d+)\.(?P<minor>\d+)(?:\.(?P<patch>\d+))?")
                .unwrap();
    }

    fn to_u64(opt: regex::Match) -> Option<u64> {
        opt.as_str().parse().ok()
    }

    let caps: regex::Captures = VERSION_PATTERN.captures(raw)?;
    Some(semver::Version::new(
        caps.name("major").and_then(to_u64)?,
        caps.name("minor").and_then(to_u64)?,
        caps.name("patch").and_then(to_u64).unwrap_or(0),
    ))
}

#[derive(Error, Debug)]
pub enum VersionError {
    #[error("could not extract version data from output:\n{0}")]
    UnknownFormat(String),
    #[error("fping was not found in FPING_BIN or PATH")]
    BinaryNotFound,
    #[error("libc failure, required file /etc/protocols missing")]
    DependenciesMissing,
    #[error("unknown fping exit code: {:?}\n{1}", .0.code())]
    ProcessFailure(ExitStatus, String),
    #[error("unknown io failure")]
    Other(#[source] io::Error),
    #[error("{0}")]
    SpecificFailure(String),
}

impl From<io::Error> for VersionError {
    fn from(e: io::Error) -> Self {
        match e.kind() {
            io::ErrorKind::NotFound => VersionError::BinaryNotFound,
            _ => VersionError::Other(e),
        }
    }
}

pub(crate) fn output_to_version(
    output: io::Result<Output>,
) -> Result<semver::Version, VersionError> {
    let output = output?;
    match output.status.code() {
        Some(0) => {
            let raw = std::str::from_utf8(&output.stdout).unwrap();
            parse_fping_version(raw).ok_or_else(|| VersionError::UnknownFormat(raw.to_string()))
        }
        Some(4) => Err(VersionError::DependenciesMissing),
        _ => Err(VersionError::ProcessFailure(
            output.status,
            String::from_utf8(output.stdout).unwrap(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::parse_fping_version;

    #[test]
    fn handling_fping_paths() {
        fn basic_template(program_path: &str) {
            assert_eq!(
                parse_fping_version(&format!(
                    "{0}: Version 4.2\n{0}: comments to david@schweikert.ch\n",
                    program_path
                )),
                Some(Version::new(4, 2, 0))
            );
            assert_eq!(
                parse_fping_version(&format!("{}: Version 5.0\n", program_path)),
                Some(Version::new(5, 0, 0))
            );
        }

        // Lookup through PATH
        basic_template("fping");
        // Direct call
        basic_template("/bin/fping");
        // nix derivation
        basic_template("/nix/store/s03vfmkr85irmca739szvnpfrps267pl-fping-5.0/bin/fping");
        // relative call
        basic_template("../fping/bin/fping");

        // No output -> failure to parse
        assert_eq!(parse_fping_version(""), None);
    }
}
