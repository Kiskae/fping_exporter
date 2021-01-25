use std::{ffi::OsStr, io, process::Stdio, time::Duration};

use tokio::{
    process::{Child, Command},
    time::error::Elapsed,
};

use crate::event_stream::{EventStreamSource, PendingStream};

mod protocol;
pub mod version;

pub use protocol::{Control, Ping};

pub struct Launcher<'t> {
    program: &'t str,
}

pub fn for_program<S>(program: &S) -> Launcher
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

    pub async fn spawn<S: AsRef<OsStr>>(&self, targets: &[S]) -> io::Result<PendingStream<Child>> {
        Command::new(self.program)
            .arg("-ADln")
            .args(targets)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?
            .as_eventstream()
    }
}
