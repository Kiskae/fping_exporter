use std::time::Duration;

use tokio::{process::Command, time::error::Elapsed};

pub mod version;

pub struct Launcher<'t> {
    program: &'t str,
}

pub fn for_program<'t, S>(program: &'t S) -> Launcher<'t>
where
    S: AsRef<str> + ?Sized,
{
    Launcher {
        program: program.as_ref(),
    }
}

impl From<Elapsed> for version::VersionError {
    fn from(_: Elapsed) -> Self {
        Self::SpecificFailure("fping failed to exit in a reasonable timespan, please ensure FPING_BIN points to a valid version of fping".to_string())
    }
}

impl<'t> Launcher<'t> {
    pub async fn version(
        &self,
        timeout: Duration,
    ) -> Result<semver::Version, version::VersionError> {
        version::output_to_version(
            tokio::time::timeout(
                timeout,
                Command::new(self.program)
                    .arg("--version")
                    .kill_on_drop(true)
                    .output(),
            )
            .await?,
        )
    }
}
