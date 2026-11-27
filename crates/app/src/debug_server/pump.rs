//! Hands commands from client threads to the GPUI main thread. The foreground
//! task sleeps on a stored `Waker` until `submit` wakes it, so an idle server
//! costs no timer polling.
//!
//! Input commands and resizes are answered only after the window has painted their effect
//! (two `on_next_frame` hops: the first runs at the start of the next frame
//! request, before that frame draws; the second after it). The client's reader
//! thread does not read its next request until then, so a script can never act
//! on a stale screen.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use gpui_kit::{App, Entity, WeakEntity};
use jade_debug_protocol::{Answer, Command, Message, ReqId};

use super::client::Client;
use super::{capture, commands, input};
use crate::JadeApp;

/// Commands finished so far; the headless loop paints a frame after each.
pub(super) static COMPLETED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Set when `quit` was answered, for a loop that has to stop itself.
pub(super) static QUIT: AtomicBool = AtomicBool::new(false);

/// Commands waiting for the main thread; beyond this a client is told `busy`.
const MAX_QUEUED: usize = 256;

/// Answers one request exactly once, from whichever side gets there first
/// (the main thread, or the reader giving up after its timeout).
#[derive(Clone)]
pub(super) struct Reply(Arc<ReplyInner>);

struct ReplyInner {
    client: Arc<Client>,
    req: ReqId,
    sent: AtomicBool,
}

impl Reply {
    pub(super) fn new(client: Arc<Client>, req: ReqId) -> Self {
        Self(Arc::new(ReplyInner {
            client,
            req,
            sent: AtomicBool::new(false),
        }))
    }

    pub(super) fn send(&self, answer: Answer) {
        self.send_then(answer, Vec::new());
    }

    /// Sends `answer`, then `follow` (chunks of a capture, stamped with this
    /// request's id) in order.
    pub(super) fn send_then(&self, answer: Answer, follow: Vec<Message>) {
        if self.0.sent.swap(true, Ordering::AcqRel) {
            return;
        }
        if !self.0.client.reply(self.0.req, answer) {
            return;
        }
        for mut message in follow {
            if let Message::Chunk { req, .. } = &mut message {
                *req = self.0.req;
            }
            if !self.0.client.send(&message) {
                return;
            }
        }
    }
}

pub(super) struct Job {
    pub reply: Reply,
    pub command: Command,
    /// Signalled once the command is fully finished (for input: painted).
    pub done: Sender<()>,
}

struct Shared {
    queue: VecDeque<Job>,
    waker: Option<Waker>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    queue: VecDeque::new(),
    waker: None,
});

pub(super) fn submit(job: Job) {
    let waker = {
        let Ok(mut shared) = SHARED.lock() else {
            return;
        };
        if shared.queue.len() >= MAX_QUEUED {
            drop(shared);
            job.reply.send(Answer::Error {
                message: "busy: command queue full".into(),
            });
            let _ = job.done.send(());
            return;
        }
        shared.queue.push_back(job);
        shared.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

struct NextJob;

impl Future for NextJob {
    type Output = Job;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Job> {
        let Ok(mut shared) = SHARED.lock() else {
            return Poll::Pending;
        };
        match shared.queue.pop_front() {
            Some(job) => Poll::Ready(job),
            None => {
                shared.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// Runs one job; input that succeeded is answered after the next paint.
fn run(job: Job, app: &WeakEntity<JadeApp>, cx: &mut App) {
    let Job {
        reply,
        command,
        done,
    } = job;
    let quit = matches!(command, Command::Quit);
    let wait_for_paint = input::is_input(&command) || matches!(command, Command::Resize { .. });
    if capture::is_pixel_command(&command) {
        let (answer, follow) = capture::execute(&command, cx);
        reply.send_then(answer, follow);
        let _ = done.send(());
        COMPLETED.fetch_add(1, Ordering::AcqRel);
        return;
    }
    let answer = commands::execute(command, app, cx);
    let window = cx.windows().into_iter().next();
    match window {
        Some(window) if wait_for_paint && answer == Answer::Ok => {
            let finish = move |_: &mut gpui_kit::Window, _: &mut App| {
                reply.send(Answer::Ok);
                let _ = done.send(());
            };
            let hopped = window.update(cx, |_, window, _| {
                window.on_next_frame(move |window, _| window.on_next_frame(finish));
            });
            if hopped.is_err() {
                eprintln!("[jade-debug] window vanished before its reply could be scheduled");
            }
        }
        _ => {
            reply.send(answer);
            let _ = done.send(());
        }
    }
    COMPLETED.fetch_add(1, Ordering::AcqRel);
    if quit {
        super::remove_socket();
        QUIT.store(true, Ordering::Release);
        cx.quit();
    }
}

/// Starts the main-thread task that runs submitted commands. No-op unless the
/// debug server was requested.
pub(crate) fn install(app: &Entity<JadeApp>, cx: &mut App) {
    if !super::requested() {
        return;
    }
    let app: WeakEntity<JadeApp> = app.downgrade();
    cx.spawn(async move |cx| {
        loop {
            let job = NextJob.await;
            cx.update(|cx| run(job, &app, cx));
        }
    })
    .detach();
}
