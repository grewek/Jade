//! A blocking protocol client for tests.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use jade_debug_protocol::{Answer, Command, Message, ReqId, Request, decode_line, encode_line};

/// Starts a server on a fresh socket. Tests run with `--test-threads=1`.
pub(crate) fn start_test_server() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "jade-debug-{}-{:?}.sock",
        std::process::id(),
        std::thread::current().id()
    ));
    // SAFETY: tests run single-threaded (`--test-threads=1`).
    unsafe { std::env::set_var("JADE_DEBUG_SOCKET", &path) };
    super::start_if_enabled();
    path
}

const RECV_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) struct TestClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_req: ReqId,
}

impl TestClient {
    pub(crate) fn connect(path: &Path) -> Self {
        let writer = UnixStream::connect(path).expect("connect to the debug socket");
        let reader = BufReader::new(writer.try_clone().expect("clone the stream"));
        Self {
            reader,
            writer,
            next_req: 1,
        }
    }

    pub(crate) fn send(&mut self, command: Command) -> ReqId {
        let req = self.next_req;
        self.next_req += 1;
        let line = encode_line(&Request { req, command }).expect("encode");
        self.send_raw(line.as_bytes());
        req
    }

    pub(crate) fn send_raw(&mut self, bytes: &[u8]) {
        // The server may drop us mid-write on purpose (oversized line).
        let _ = self.writer.write_all(bytes);
    }

    /// Next message, or `None` after `timeout` (or if the server hung up).
    pub(crate) fn try_recv(&mut self, timeout: Duration) -> Option<Message> {
        self.reader.get_ref().set_read_timeout(Some(timeout)).ok()?;
        let mut line = Vec::new();
        self.reader
            .read_until(b'\n', &mut line)
            .ok()
            .filter(|&n| n > 0)?;
        Some(decode_line(std::str::from_utf8(&line).ok()?).expect("a valid message"))
    }

    pub(crate) fn recv(&mut self) -> Message {
        self.try_recv(RECV_TIMEOUT)
            .expect("a message within the timeout")
    }

    /// Sends `command` and returns the answer to it, skipping pushed events.
    pub(crate) fn call(&mut self, command: Command) -> Answer {
        let req = self.send(command);
        loop {
            if let Message::Response { req: got, answer } = self.recv()
                && got == req
            {
                return answer;
            }
        }
    }

    /// Whether the server has closed the connection.
    pub(crate) fn is_closed(&mut self) -> bool {
        let deadline = Instant::now() + RECV_TIMEOUT;
        while Instant::now() < deadline {
            let mut buf = Vec::new();
            let _ = self
                .reader
                .get_ref()
                .set_read_timeout(Some(Duration::from_millis(50)));
            if let Ok(0) = self.reader.read_until(b'\n', &mut buf) {
                return true;
            }
        }
        false
    }

    /// Polls `condition` for up to the receive timeout; server state changes
    /// land on other threads shortly after the message that causes them.
    pub(crate) fn wait_until(&self, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + RECV_TIMEOUT;
        while !condition() {
            assert!(Instant::now() < deadline, "condition not met in time");
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
