#![deny(warnings, rust_2018_idioms)]

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
pub struct AbortableStream<S, T, E> {
    stream: S,
    err_handler: Option<fn(E) -> Option<T>>,
    stop: bool,
}

impl<S, T, E> AbortableStream<S, T, E> {
    /// Creates a new instance, wrapping the provided stream and using the provided callback to
    /// convert errors before outputting them.
    pub fn new_err(stream: S, err_handler: fn(E) -> Option<T>) -> AbortableStream<S, T, E> {
        AbortableStream {
            stream,
            err_handler: Some(err_handler),
            stop: false,
        }
    }

    /// Creates a new instance, wrapping the provided stream and passing through received errors
    /// directly.
    pub fn new(stream: S) -> AbortableStream<S, T, E> {
        AbortableStream {
            stream,
            err_handler: None,
            stop: false,
        }
    }
}

impl<S, T, E> Stream for AbortableStream<S, T, E>
where
    S: Stream<Item = AbortableItem<T>, Error = E>,
{
    type Item = T;
    type Error = E;

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
            Err(err) => {
                // Use custom handler, or fall back to just forwarding the error as-is.
                match self.err_handler {
                    Some(err_handler) => Ok(Async::Ready(err_handler(err))),
                    None => Err(err),
                }
            }
        }
    }
}
