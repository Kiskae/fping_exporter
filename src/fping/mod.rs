use tokio::process::Command;

mod version;

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

impl<'t> Launcher<'t> {
    pub async fn version(&self) -> Result<semver::Version, version::VersionParseError> {
        version::output_to_version(
            Command::new(self.program)
                .arg("--version")
                .kill_on_drop(true)
                .output()
                .await,
        )
    }
}
