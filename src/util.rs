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

    #[derive(Debug)]
    pub struct Interrupted<T>(pub T);

    impl<F, H> ControlToInterrupt<F, H::Signal>
    where
        F: EventHandler<Handle = H>,
        H: Interruptable,
    {
        pub fn new(handler: F, signal: H::Signal) -> Self {
            Self { handler, signal }
        }
    }

    impl<F, S, T> EventHandler for ControlToInterrupt<F, S>
    where
        F: EventHandler<Token = Interrupted<T>>,
        S: Copy + std::fmt::Debug,
        F::Handle: Interruptable<Signal = S> + std::fmt::Debug,
    {
        type Output = F::Output;
        type Error = F::Error;
        type Handle = F::Handle;
        type Token = T;

        fn on_output(&mut self, event: Self::Output) {
            self.handler.on_output(event)
        }

        fn on_error(&mut self, event: Self::Error) {
            self.handler.on_error(event)
        }

        fn on_control(
            &mut self,
            handle: &mut Self::Handle,
            token: Self::Token,
        ) -> std::io::Result<()> {
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

    impl<F, T> EventHandler for LockControl<F>
    where
        F: EventHandler<Token = (T, Claim)>,
        T: std::fmt::Debug,
    {
        type Output = F::Output;
        type Error = F::Error;
        type Handle = F::Handle;
        type Token = T;

        fn on_output(&mut self, event: Self::Output) {
            self.handler.on_output(event)
        }

        fn on_error(&mut self, event: Self::Error) {
            self.handler.on_error(event)
        }

        fn on_control(
            &mut self,
            handle: &mut Self::Handle,
            token: Self::Token,
        ) -> std::io::Result<()> {
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
