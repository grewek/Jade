//! A blocking connection to the app's debug socket.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use jade_debug_protocol::{
    Answer, Command, Event, MAX_LINE_BYTES, Message, ReqId, Request, decode_line, encode_line,
};

/// How long any single reply may take (an `await` adds its own timeout).
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct Connection {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_req: ReqId,
}

impl Connection {
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let stream = UnixStream::connect(path).map_err(|err| {
            format!(
                "cannot connect to {}: {err}\n  is the app running with JADE_DEBUG_SOCKET set? \
                 `jade-debug launch <vault>` starts one",
                path.display()
            )
        })?;
        Self::from_stream(stream)
    }

    pub(crate) fn from_stream(stream: UnixStream) -> Result<Self, String> {
        let reader = stream.try_clone().map_err(|err| err.to_string())?;
        Ok(Self {
            reader: BufReader::new(reader),
            writer: stream,
            next_req: 1,
        })
    }

    /// Sends `command` and returns its answer, plus the events that arrived first.
    pub(crate) fn call(&mut self, command: Command) -> Result<(Answer, Vec<Event>), String> {
        let wait = match &command {
            Command::Await { timeout_ms, .. } => Duration::from_millis(u64::from(*timeout_ms)),
            _ => Duration::ZERO,
        };
        let req = self.next_req;
        self.next_req += 1;
        let line = encode_line(&Request { req, command }).map_err(|err| err.to_string())?;
        self.writer
            .write_all(line.as_bytes())
            .map_err(|err| format!("send failed: {err}"))?;

        let deadline = Instant::now() + CALL_TIMEOUT + wait;
        let mut events = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.next_message(left)? {
                None => return Err("timed out waiting for the app to answer".into()),
                Some(Message::Event { event }) => events.push(event),
                Some(Message::Response { req: got, answer }) if got == req => {
                    return Ok((answer, events));
                }
                // Request 0 is the app rejecting a line it could not parse.
                Some(Message::Response { req: 0, answer }) => return Ok((answer, events)),
                Some(Message::Response { .. }) => {} // a stale answer
            }
        }
    }

    /// The next message, `None` if `timeout` passes first.
    pub(crate) fn next_message(&mut self, timeout: Duration) -> Result<Option<Message>, String> {
        let timeout = timeout.max(Duration::from_millis(1));
        self.reader
            .get_ref()
            .set_read_timeout(Some(timeout))
            .map_err(|err| err.to_string())?;
        let mut line = Vec::new();
        match self.reader.read_until(b'\n', &mut line) {
            Ok(0) => Err("the app closed the connection".into()),
            Ok(_) if line.len() > MAX_LINE_BYTES => Err("the app sent an oversized line".into()),
            Ok(_) => {
                let text = std::str::from_utf8(&line).map_err(|err| err.to_string())?;
                decode_line(text)
                    .map(Some)
                    .map_err(|err| format!("bad message: {err:?}"))
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(err) => Err(err.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jade_debug_protocol::{FrameSample, Topic};

    /// A fake app: answers each request with `reply(req)`, optionally after
    /// pushing events, until the client hangs up.
    fn fake_app(
        mut reply: impl FnMut(Request) -> Vec<Message> + Send + 'static,
    ) -> (Connection, std::thread::JoinHandle<()>) {
        let (client, server) = UnixStream::pair().unwrap();
        let handle = std::thread::spawn(move || {
            let mut reader = BufReader::new(server.try_clone().unwrap());
            let mut writer = server;
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 0) {
                let request: Request = decode_line(&line).unwrap();
                for message in reply(request) {
                    writer
                        .write_all(encode_line(&message).unwrap().as_bytes())
                        .unwrap();
                }
                line.clear();
            }
        });
        (Connection::from_stream(client).unwrap(), handle)
    }

    fn frames_event() -> Event {
        Event::Frames(FrameSample {
            frame: 1,
            render_us: 2,
            paint_us: 3,
            interval_us: 4,
        })
    }

    #[test]
    fn a_call_returns_its_answer_and_the_events_that_came_first() {
        let (mut conn, app) = fake_app(|r| {
            vec![
                Message::Event {
                    event: frames_event(),
                },
                Message::Response {
                    req: r.req + 100,
                    answer: Answer::Error {
                        message: "stale".into(),
                    },
                },
                Message::Response {
                    req: r.req,
                    answer: Answer::Ok,
                },
            ]
        });
        let (answer, events) = conn.call(Command::ListTargets).unwrap();
        assert_eq!(answer, Answer::Ok);
        assert_eq!(events, vec![frames_event()]);
        let (answer, _) = conn.call(Command::Quit).unwrap();
        assert_eq!(answer, Answer::Ok, "request ids advance per call");
        drop(conn);
        app.join().unwrap();
    }

    #[test]
    fn a_rejected_line_surfaces_as_its_error() {
        let (mut conn, app) = fake_app(|_| {
            vec![Message::Response {
                req: 0,
                answer: Answer::Error {
                    message: "bad json".into(),
                },
            }]
        });
        let (answer, _) = conn.call(Command::ListTargets).unwrap();
        assert_eq!(
            answer,
            Answer::Error {
                message: "bad json".into()
            }
        );
        drop(conn);
        app.join().unwrap();
    }

    #[test]
    fn next_message_times_out_quietly_and_reports_a_closed_app() {
        let (mut conn, app) = fake_app(|_| Vec::new());
        assert!(
            conn.next_message(Duration::from_millis(20))
                .unwrap()
                .is_none()
        );
        let (_conn_b, app_b) = fake_app(|_| Vec::new());
        drop(_conn_b);
        app_b.join().unwrap();
        drop(conn);
        app.join().unwrap();

        let (client, server) = UnixStream::pair().unwrap();
        drop(server);
        let mut closed = Connection::from_stream(client).unwrap();
        assert!(
            closed
                .next_message(Duration::from_millis(50))
                .unwrap_err()
                .contains("closed")
        );
    }

    #[test]
    fn await_extends_the_deadline_and_returns_the_event_answer() {
        let (mut conn, app) = fake_app(|r| {
            vec![Message::Response {
                req: r.req,
                answer: Answer::Event {
                    event: frames_event(),
                },
            }]
        });
        let (answer, _) = conn
            .call(Command::Await {
                topic: Topic::Frames,
                timeout_ms: 10,
            })
            .unwrap();
        assert_eq!(
            answer,
            Answer::Event {
                event: frames_event()
            }
        );
        drop(conn);
        app.join().unwrap();
    }
}
