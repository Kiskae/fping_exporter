pub mod signal {
    use std::io;

    use crate::event_stream::EventHandler;

    pub trait Interruptable {
        type Signal;

        fn interrupt(&mut self, signal: Self::Signal) -> io::Result<bool>;
    }

    #[cfg(unix)]
    impl Interruptable for tokio::process::Child {
        type Signal = nix::sys::signal::Signal;

        fn interrupt(&mut self, signal: Self::Signal) -> io::Result<bool> {
            use nix::{sys::signal, unistd::Pid};

            fn map_errno(err: nix::Error) -> io::Error {
                match err {
                    nix::Error::Sys(no) => io::Error::from_raw_os_error(no as i32),
                    _ => io::ErrorKind::Other.into(),
                }
            }

            signal::kill(
                Pid::from_raw(self.id().ok_or(io::ErrorKind::BrokenPipe)? as i32),
                signal,
            )
            .map_err(map_errno)
            .map(|_| true)
        }
    }

    pub struct ControlToInterrupt<F, S> {
        handler: F,
        signal: S,
    }

    pub fn apply<F, O, E, H, T>(signal: H::Signal, handler: F) -> ControlToInterrupt<F, H::Signal>
    where
        H: Interruptable,
        F: EventHandler<O, E, H, T>,
    {
        ControlToInterrupt { handler, signal }
    }

    #[derive(Debug)]
    pub struct Interrupted<T>(pub T);

    impl<F, O, E, H, T> EventHandler<O, E, H, T> for ControlToInterrupt<F, H::Signal>
    where
        H: Interruptable + std::fmt::Debug,
        H::Signal: Copy + std::fmt::Debug,
        F: EventHandler<O, E, H, Interrupted<T>>,
    {
        fn on_output(&mut self, event: O) {
            self.handler.on_output(event)
        }

        fn on_error(&mut self, event: E) {
            self.handler.on_error(event)
        }

        fn on_control(&mut self, handle: &mut H, token: T) -> io::Result<()> {
            if handle.interrupt(self.signal)? {
                self.handler.on_control(handle, Interrupted(token))
            } else {
                debug!("failed to send {:?} to {:?}", self.signal, handle);
                Ok(())
            }
        }
    }
}

pub mod lock {
    use std::sync::Arc;

    use log::debug;
    use tokio::sync::{Mutex, OwnedMutexGuard};

    use crate::event_stream::EventHandler;

    #[derive(Debug)]
    pub struct Claim {
        inner: OwnedMutexGuard<()>,
    }

    #[derive(Debug)]
    pub struct LockControl<F> {
        handler: F,
        lock: Arc<Mutex<()>>,
    }

    impl<F> LockControl<F> {
        pub fn new(handler: F) -> Self {
            LockControl {
                handler,
                lock: Arc::new(Mutex::new(())),
            }
        }
    }

    impl<F, O, E, H, T: std::fmt::Debug> EventHandler<O, E, H, T> for LockControl<F>
    where
        F: EventHandler<O, E, H, (T, Claim)>,
    {
        fn on_output(&mut self, event: O) {
            self.handler.on_output(event)
        }

        fn on_error(&mut self, event: E) {
            self.handler.on_error(event)
        }

        fn on_control(&mut self, handle: &mut H, token: T) -> std::io::Result<()> {
            if let Ok(lock) = self.lock.clone().try_lock_owned() {
                self.handler
                    .on_control(handle, (token, Claim { inner: lock }))
            } else {
                debug!("try-lock failed, dropping {:?}", token);
                Ok(())
            }
        }
    }
}
