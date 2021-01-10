#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate clap;

use std::env;

use cfg_if::cfg_if;

mod args;
mod fping;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let fping_binary = env::var("FPING_BIN").unwrap_or("fping".into());
    let launcher = fping::for_program(&fping_binary);
    let args = args::load_args(&launcher).await?;
    println!("{:?}", args);
    Ok(())
}
