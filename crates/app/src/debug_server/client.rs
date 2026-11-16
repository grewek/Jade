//! One connected debug client: a reader thread decoding requests, plus the
//! state `emit` needs to route pushed events (subscriptions, pending awaits).

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use jade_debug_protocol::{
    Answer, Command, Event, MAX_LINE_BYTES, Message, Query, ReqId, Request, Topic, decode_line,
    encode_line,
};

use super::{pump, topic_bit};

/// How often the reader wakes to expire timed-out awaits.
const TICK: Duration = Duration::from_millis(50);
/// A stuck client must not stall the emitting thread.
const WRITE_TIMEOUT: Duration = Duration::from_millis(200);
const MAX_WAITERS: usize = 16;

struct Waiter {
    req: ReqId,
    topic: Topic,
    deadline: Instant,
}

#[derive(Default)]
struct State {
    subscribed: u8,
    waiters: Vec<Waiter>,
}

pub(super) struct Client {
    writer: Mutex<UnixStream>,
    state: Mutex<State>,
    dead: AtomicBool,
}

impl Client {
    pub(super) fn new(stream: UnixStream) -> std::io::Result<Self> {
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        Ok(Self {
            writer: Mutex::new(stream),
            state: Mutex::new(State::default()),
            dead: AtomicBool::new(false),
        })
    }

    pub(super) fn is_dead(&self) -> bool {
        self.dead.load(Ordering::Relaxed)
    }

    /// Topics this client currently needs produced.
    pub(super) fn wanted_mask(&self) -> u8 {
        let Ok(state) = self.state.lock() else {
            return 0;
        };
        state
            .waiters
            .iter()
            .fold(state.subscribed, |mask, w| mask | topic_bit(w.topic))
    }

    /// Writes one already-encoded line; a failed write marks the client dead.
    pub(super) fn send_line(&self, line: &str) -> bool {
        let Ok(mut writer) = self.writer.lock() else {
            return false;
        };
        if let Err(err) = writer.write_all(line.as_bytes()) {
            eprintln!(
                "[jade-debug] dropped a client ({} byte line): {err}",
                line.len()
            );
            self.dead.store(true, Ordering::Relaxed);
            return false;
        }
        true
    }

    pub(super) fn send(&self, message: &Message) -> bool {
        match encode_line(message) {
            Ok(line) => self.send_line(&line),
            Err(err) => {
                eprintln!("[jade-debug] cannot encode a reply: {err}");
                false
            }
        }
    }

    pub(super) fn reply(&self, req: ReqId, answer: Answer) -> bool {
        self.send(&Message::Response { req, answer })
    }

    /// Pushes `event` (pre-encoded as `event_line`) to a subscribed client and
    /// answers any `Await` on its topic. Returns false once the client is dead.
    pub(super) fn deliver(&self, event: &Event, event_line: &str) -> bool {
        let topic = event.topic();
        let (subscribed, resolved) = {
            let Ok(mut state) = self.state.lock() else {
                return false;
            };
            let (hit, rest) = std::mem::take(&mut state.waiters)
                .into_iter()
                .partition(|w| w.topic == topic);
            state.waiters = rest;
            (state.subscribed & topic_bit(topic) != 0, hit)
        };
        if subscribed {
            self.send_line(event_line);
        }
        for waiter in resolved {
            let event = event.clone();
            self.reply(waiter.req, Answer::Event { event });
        }
        !self.is_dead()
    }

    fn set_subscribed(&self, topic: Topic, on: bool) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if on {
            state.subscribed |= topic_bit(topic);
        } else {
            state.subscribed &= !topic_bit(topic);
        }
    }

    fn add_waiter(&self, waiter: Waiter) -> Result<(), &'static str> {
        let Ok(mut state) = self.state.lock() else {
            return Err("client state poisoned");
        };
        if state.waiters.len() >= MAX_WAITERS {
            return Err("too many pending awaits");
        }
        state.waiters.push(waiter);
        Ok(())
    }

    fn expire_waiters(&self, now: Instant) {
        let expired = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            let (expired, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut state.waiters)
                .into_iter()
                .partition(|w| w.deadline <= now);
            state.waiters = rest;
            expired
        };
        if !expired.is_empty() {
            super::refresh_wanted(); // an expired await no longer wants its topic
        }
        for waiter in expired {
            let message = format!("timed out waiting for a {:?} event", waiter.topic);
            self.reply(waiter.req, Answer::Error { message });
        }
    }
}

/// Reads requests until the client disconnects or misbehaves.
pub(super) fn serve(client: &Arc<Client>, stream: UnixStream) {
    if stream.set_read_timeout(Some(TICK)).is_err() {
        return;
    }
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    while !client.is_dead() {
        match read_chunk(&mut reader, &mut line) {
            Chunk::Line => {
                handle_line(client, &line);
                line.clear();
            }
            Chunk::Pending => {}
            Chunk::Closed => return,
        }
        client.expire_waiters(Instant::now());
    }
}

enum Chunk {
    Line,
    Pending,
    Closed,
}

/// Appends to `line` up to a newline, never past `MAX_LINE_BYTES`.
fn read_chunk(reader: &mut BufReader<UnixStream>, line: &mut Vec<u8>) -> Chunk {
    assert!(line.len() <= MAX_LINE_BYTES);
    let room = (MAX_LINE_BYTES + 1 - line.len()) as u64;
    match reader.by_ref().take(room).read_until(b'\n', line) {
        Ok(0) => Chunk::Closed,
        Ok(_) if line.ends_with(b"\n") => Chunk::Line,
        Ok(_) if line.len() > MAX_LINE_BYTES => {
            eprintln!("[jade-debug] dropped a client: line over {MAX_LINE_BYTES} bytes");
            Chunk::Closed
        }
        Ok(_) => Chunk::Pending, // EOF mid-line; the next read reports it
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Chunk::Pending,
        Err(err) if err.kind() == std::io::ErrorKind::TimedOut => Chunk::Pending,
        Err(_) => Chunk::Closed,
    }
}

fn handle_line(client: &Arc<Client>, line: &[u8]) {
    let parsed = std::str::from_utf8(line)
        .map_err(|err| err.to_string())
        .and_then(|text| decode_line::<Request>(text).map_err(|err| format!("{err:?}")));
    match parsed {
        Ok(request) => dispatch(client, request),
        Err(message) => {
            client.reply(0, Answer::Error { message });
        }
    }
}

fn dispatch(client: &Arc<Client>, request: Request) {
    let req = request.req;
    match request.command {
        Command::Subscribe { topic } => {
            client.set_subscribed(topic, true);
            super::refresh_wanted();
            client.reply(req, Answer::Ok);
        }
        Command::Unsubscribe { topic } => {
            client.set_subscribed(topic, false);
            super::refresh_wanted();
            client.reply(req, Answer::Ok);
        }
        Command::Await { topic, timeout_ms } => {
            let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
            match client.add_waiter(Waiter {
                req,
                topic,
                deadline,
            }) {
                Ok(()) => super::refresh_wanted(),
                Err(message) => {
                    client.reply(
                        req,
                        Answer::Error {
                            message: message.into(),
                        },
                    );
                }
            }
        }
        Command::Query {
            what: Query::Memory,
        } => {
            let answer = match crate::memory_probe::sample() {
                Some(info) => Answer::Memory(info),
                None => Answer::Error {
                    message: "memory unavailable on this platform".into(),
                },
            };
            client.reply(req, answer);
        }
        command => pump::submit(pump::Job {
            client: Arc::clone(client),
            req,
            command,
        }),
    }
}
