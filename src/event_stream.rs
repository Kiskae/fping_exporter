use std::{future, io};

use tokio::{
    io::{AsyncRead, BufReader, Lines},
    process::Child,
    sync::mpsc,
};

#[derive(Debug)]
pub enum Event<O, E, C> {
    Output(O),
    Error(E),
    Control(C),
}

pub struct PendingStream<ES: EventStreamSource + ?Sized> {
    handle: ES::Handle,
    stdout: Option<Lines<BufReader<ES::Stdout>>>,
    stderr: Option<Lines<BufReader<ES::Stderr>>>,
    interrupts: Option<mpsc::Receiver<()>>,
}

impl<ES: EventStreamSource> PendingStream<ES> {
    pub fn create(
        handle: ES::Handle,
        stdout: Option<ES::Stdout>,
        stderr: Option<ES::Stderr>,
    ) -> Self {
        use tokio::io::AsyncBufReadExt;
        PendingStream {
            handle,
            stdout: stdout.map(BufReader::new).map(AsyncBufReadExt::lines),
            stderr: stderr.map(BufReader::new).map(AsyncBufReadExt::lines),
            interrupts: None,
        }
    }

    pub fn inner(self) -> ES::Handle {
        self.handle
    }
}

impl<ES: EventStreamSource> PendingStream<ES> {
    pub async fn listen(
        &mut self,
        mut handler: impl FnMut(Event<&str, &str, ()>),
    ) -> io::Result<()> {
        async fn optional_call<T, F, O>(opt: Option<T>, async_fn: impl FnOnce(T) -> F) -> Option<O>
        where
            F: future::Future<Output = Option<O>>,
        {
            match opt {
                Some(x) => async_fn(x).await,
                None => None,
            }
        }

        async fn get_line<R>(lines: &mut Lines<R>) -> Option<io::Result<String>>
        where
            R: tokio::io::AsyncBufRead + Unpin,
        {
            lines.next_line().await.transpose()
        }

        loop {
            //TODO: inline transform
            //TODO: control structure
            tokio::select! {
                Some(_) = optional_call(self.interrupts.as_mut(), mpsc::Receiver::recv) => {
                    //TODO: delegate behavior to stateful typevar
                    handler(Event::Control(()))
                }
                Some(out) = optional_call(self.stdout.as_mut(), get_line) => {
                    handler(Event::Output(&out?))
                }
                Some(err) = optional_call(self.stderr.as_mut(), get_line) => {
                    handler(Event::Error(&err?))
                }
                else => {
                    break;
                }
            }
        }

        Ok(())
    }
}

pub trait EventStreamSource {
    type Handle;
    type Stdout: AsyncRead + Unpin;
    type Stderr: AsyncRead + Unpin;

    fn as_eventstream(self) -> io::Result<PendingStream<Self>>;
}

impl EventStreamSource for Child {
    type Handle = Self;
    type Stdout = tokio::process::ChildStdout;
    type Stderr = tokio::process::ChildStderr;

    fn as_eventstream(mut self) -> io::Result<PendingStream<Self>> {
        let stdout = self.stdout.take();
        let stderr = self.stderr.take();
        Ok(PendingStream::create(self, stdout, stderr))
    }
}

//Temporary allow, used for testing
#[allow(dead_code)]
mod synthetic {
    use std::io;

    use tokio::io::AsyncRead;

    use super::{EventStreamSource, PendingStream};

    enum SyntheticStream<S: AsyncRead + Unpin> {
        Stdout(S),
        Stderr(S),
    }

    impl<S> super::EventStreamSource for SyntheticStream<S>
    where
        S: AsyncRead + Unpin,
    {
        type Handle = ();
        type Stdout = S;
        type Stderr = S;

        fn as_eventstream(self) -> io::Result<super::PendingStream<Self>> {
            let (stdout, stderr) = match self {
                SyntheticStream::Stdout(s) => (Some(s), None),
                SyntheticStream::Stderr(s) => (None, Some(s)),
            };
            Ok(PendingStream::create((), stdout, stderr))
        }
    }

    pub fn as_stdout<S: AsyncRead + Unpin>(
        stream: S,
    ) -> io::Result<super::PendingStream<impl EventStreamSource>> {
        SyntheticStream::Stdout(stream).as_eventstream()
    }

    pub fn as_stderr<S: AsyncRead + Unpin>(
        stream: S,
    ) -> io::Result<super::PendingStream<impl EventStreamSource>> {
        SyntheticStream::Stderr(stream).as_eventstream()
    }
}

pub use synthetic::{as_stderr, as_stdout};
