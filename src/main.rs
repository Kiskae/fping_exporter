#[macro_use]
extern crate lazy_static;

use std::{env, time::Duration};

use tokio::time::timeout;

mod fping;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let fping_binary = env::var("FPING_BIN").unwrap_or("fping".into());
    let launcher = fping::for_program(&fping_binary);
    //TODO: add timeout on launcher.version() to handle weird inputs (symlink fping to yes)
    let fping_version = timeout(Duration::from_millis(50), launcher.version()).await?;
    println!("{:?}", fping_version);
    Ok(())
}
