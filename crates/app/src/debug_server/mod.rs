//! Opt-in debug server: with `JADE_DEBUG_SOCKET=<path>` the app listens on a
//! Unix socket and speaks `jade-debug-protocol` (newline-delimited JSON). A
//! client sends requests, subscribes to event topics (paint trace, memory) and
//! the app pushes matching events, so a tool can drive and observe the live app
//! without scraping a terminal. Nothing is sent until a client subscribes.

#[cfg(test)]
mod acceptance;
mod capture;
mod client;
mod commands;
pub(crate) mod frames;
#[cfg(feature = "debug-socket")]
pub(crate) mod headless;
mod input;
mod invariants;
mod pixels;
mod pump;
pub(crate) mod targets;
#[cfg(test)]
pub(crate) mod test_client;

use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use jade_debug_protocol::{Event, Message, Topic, encode_line};

use client::Client;
pub(crate) use pump::install;

const ENV_VAR: &str = "JADE_DEBUG_SOCKET";

static CLIENTS: Mutex<Vec<Arc<Client>>> = Mutex::new(Vec::new());
/// Union of every client's topics, so producers skip work nobody wants.
static WANTED: AtomicU8 = AtomicU8::new(0);

fn topic_bit(topic: Topic) -> u8 {
    1 << (topic as u8)
}

/// Whether the socket was requested; lets producers skip work otherwise.
pub(crate) fn requested() -> bool {
    std::env::var_os(ENV_VAR).is_some()
}

/// Removes the socket file; the app is about to exit.
pub(crate) fn remove_socket() {
    if let Some(path) = std::env::var_os(ENV_VAR) {
        let _ = std::fs::remove_file(path);
    }
}

/// `requested()`, evaluated once: producers call it every frame.
pub(crate) fn enabled() -> bool {
    // Tests flip the variable between cases.
    #[cfg(test)]
    return requested();
    #[cfg(not(test))]
    {
        static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *ENABLED.get_or_init(requested)
    }
}

/// Whether any client currently wants events on `topic`.
pub(crate) fn wants(topic: Topic) -> bool {
    WANTED.load(Ordering::Relaxed) & topic_bit(topic) != 0
}

fn recompute_wanted(clients: &[Arc<Client>]) {
    let mask = clients.iter().fold(0, |mask, c| mask | c.wanted_mask());
    WANTED.store(mask, Ordering::Relaxed);
}

fn refresh_wanted() {
    if let Ok(clients) = CLIENTS.lock() {
        recompute_wanted(&clients);
    }
}

/// Starts the memory probe, then binds the socket and starts accepting
/// clients. The socket is a no-op unless requested;
/// failures are reported on stderr and leave the app running.
pub(crate) fn start_if_enabled() {
    crate::memory_probe::start_if_enabled();
    let Some(path) = std::env::var_os(ENV_VAR) else {
        return;
    };
    let _ = std::fs::remove_file(&path); // stale socket from a previous run
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(err) => return eprintln!("[jade-debug] cannot bind {path:?}: {err}"),
    };
    // Clients can inject input, so only the owner may connect.
    if let Err(err) = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)) {
        return eprintln!("[jade-debug] cannot restrict {path:?}: {err}");
    }
    eprintln!("[jade-debug] listening on {path:?}");
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || accept(stream));
        }
    });
}

fn accept(stream: std::os::unix::net::UnixStream) {
    let Ok(reader) = stream.try_clone() else {
        return;
    };
    let Ok(client) = Client::new(stream) else {
        return;
    };
    let client = Arc::new(client);
    if let Ok(mut clients) = CLIENTS.lock() {
        clients.push(Arc::clone(&client));
    }
    client::serve(&client, reader);
    if let Ok(mut clients) = CLIENTS.lock() {
        clients.retain(|c| !Arc::ptr_eq(c, &client));
        recompute_wanted(&clients);
    }
}

/// Sends `event` to every client subscribed to its topic, dropping any that
/// fail. Encodes once, and only when somebody wants it.
pub(crate) fn emit(event: &Event) {
    if !wants(event.topic()) {
        return;
    }
    let Ok(line) = encode_line(&Message::Event {
        event: event.clone(),
    }) else {
        return;
    };
    let Ok(mut clients) = CLIENTS.lock() else {
        return;
    };
    clients.retain(|c| c.deliver(event, &line));
    recompute_wanted(&clients);
}

#[cfg(test)]
mod tests {
    use super::test_client::{TestClient, start_test_server};
    use super::*;
    use jade_debug_protocol::{Answer, Command, MemoryInfo, Query};

    fn paint(frame: u64) -> Event {
        Event::Paint {
            frame,
            consistent: true,
            trace: "ok".into(),
        }
    }

    #[test]
    fn a_subscribed_client_receives_events_and_others_do_not() {
        let path = start_test_server();
        let mut client = TestClient::connect(&path);
        assert!(!wants(Topic::Paint));
        assert_eq!(
            client.call(Command::Subscribe {
                topic: Topic::Paint
            }),
            Answer::Ok
        );
        assert!(wants(Topic::Paint) && !wants(Topic::Memory));

        emit(&Event::Memory(MemoryInfo::default())); // not subscribed: dropped
        emit(&paint(1));
        assert_eq!(client.recv(), Message::Event { event: paint(1) });

        assert_eq!(
            client.call(Command::Unsubscribe {
                topic: Topic::Paint
            }),
            Answer::Ok
        );
        assert!(!wants(Topic::Paint));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn await_resolves_on_the_next_event_and_times_out_otherwise() {
        let path = start_test_server();
        let mut client = TestClient::connect(&path);
        let req = client.send(Command::Await {
            topic: Topic::Paint,
            timeout_ms: 5000,
        });
        client.wait_until(|| wants(Topic::Paint));
        emit(&paint(2));
        assert_eq!(
            client.recv(),
            Message::Response {
                req,
                answer: Answer::Event { event: paint(2) }
            }
        );
        assert!(
            !wants(Topic::Paint),
            "a resolved await stops wanting the topic"
        );

        let req = client.send(Command::Await {
            topic: Topic::Frames,
            timeout_ms: 60,
        });
        let Message::Response {
            req: got,
            answer: Answer::Error { message },
        } = client.recv()
        else {
            panic!("expected a timeout error");
        };
        assert_eq!(got, req);
        assert!(message.contains("timed out"), "{message}");
        client.wait_until(|| !wants(Topic::Frames));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn memory_queries_are_answered_on_the_client_thread() {
        let path = start_test_server();
        let mut client = TestClient::connect(&path);
        let answer = client.call(Command::Query {
            what: Query::Memory,
        });
        let Answer::Memory(info) = answer else {
            panic!("expected memory, got {answer:?}");
        };
        assert!(info.rss_kib > 0);
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn garbage_gets_an_error_and_oversized_lines_disconnect() {
        let path = start_test_server();
        let mut client = TestClient::connect(&path);
        client.send_raw(b"{not json}\n");
        let Message::Response {
            req: 0,
            answer: Answer::Error { .. },
        } = client.recv()
        else {
            panic!("expected an error reply");
        };
        assert_eq!(
            client.call(Command::Subscribe {
                topic: Topic::Memory
            }),
            Answer::Ok
        );

        client.send_raw(&vec![b'x'; jade_debug_protocol::MAX_LINE_BYTES + 10]);
        assert!(client.is_closed(), "an oversized line must drop the client");
        client.wait_until(|| !wants(Topic::Memory));
        std::fs::remove_file(&path).unwrap();
    }
}
