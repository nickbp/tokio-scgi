#![deny(warnings, rust_2018_idioms)]

use std::io::Error;
use tokio::prelude::*;

/// Type to be returned by the wrapped Stream. This tells the AbortableStream when it should avoid
/// making any additional calls to the underlying wrapped Stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AbortableItem<T> {
    /// Continue reading after this item
    Continue(T),

    /// Stop reading after this item
    Stop(T),
}

/// Wraps an underlying stream, looking for a Stop value. When Stop is observed, it will return None
/// on the next poll.
pub struct AbortableStream<S> {
    stream: S,
    stop: bool,
}

impl<S> AbortableStream<S> {
    pub fn new(stream: S) -> AbortableStream<S> {
        AbortableStream {
            stream,
            stop: false,
        }
    }
}

impl<S, T> Stream for AbortableStream<S>
where
    S: Stream<Item = AbortableItem<T>, Error = Error>,
{
    type Item = T;
    type Error = S::Error;

    fn poll(&mut self) -> Poll<Option<T>, Self::Error> {
        if self.stop {
            // Do not read from the wrapped stream, just exit.
            return Ok(Async::Ready(None));
        }
        match self.stream.poll() {
            // Interpret AbortableItem flag:
            Ok(Async::Ready(Some(AbortableItem::Continue(item)))) => Ok(Async::Ready(Some(item))),
            Ok(Async::Ready(Some(AbortableItem::Stop(item)))) => {
                self.stop = true;
                Ok(Async::Ready(Some(item)))
            }
            // Passthroughs:
            Ok(Async::Ready(None)) => Ok(Async::Ready(None)),
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(err) => Err(err),
        }
    }
}
