#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate clap;

use std::env;

use cfg_if::cfg_if;

mod args;
mod fping;

cfg_if! {
    if #[cfg(all(feature = "docker", unix))] {
        // Docker signals container shutdown through SIGTERM
        async fn terminate_signal() -> Option<()> {
            use tokio::signal::unix::{signal, SignalKind};
            signal(SignalKind::terminate()).ok()?.recv().await
        }
    } else {
        async fn terminate_signal() -> Option<()> {
            tokio::signal::ctrl_c().await.ok()
        }
    }
}

async fn fping_run() -> Option<()> {
    std::future::pending().await
}

async fn metrics_handler() -> Option<()> {
    std::future::pending().await
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let fping_binary = env::var("FPING_BIN").unwrap_or("fping".into());
    let launcher = fping::for_program(&fping_binary);
    let args = args::load_args(&launcher).await?;
    println!("{:?}", args);
    // change behavior based on args.fping_version
    //TODO: launch fping process
    //multiplex metrics/fping/cancellation
    tokio::select! {
        //TODO: terminate_signal => None -> failure to register handler
        Some(_) = terminate_signal() => {
            println!("received term")
        },
        Some(_) = fping_run() => {
            println!("fping exit")
        },
        Some(_) = metrics_handler() => {
            println!("metrics exit")
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::{
        task::JoinHandle,
        time::{error::Elapsed, timeout},
    };

    use super::*;

    async fn join_handle<H>(h: JoinHandle<H>) -> H {
        match h.await {
            Ok(x) => x,
            Err(p) => match p.try_into_panic() {
                Ok(reason) => std::panic::resume_unwind(reason),
                Err(_) => panic!("future was cancelled"),
            },
        }
    }

    #[tokio::test]
    #[cfg(feature = "docker")]
    async fn test_signal_docker() -> Result<(), Elapsed> {
        let h = tokio::spawn(terminate_signal());
        //TODO: signal sigterm to self
        assert_eq!(
            timeout(Duration::from_secs(1), join_handle(h)).await?,
            Some(())
        );
        Ok(())
    }
}
