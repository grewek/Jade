//! Hands commands from client threads to the GPUI main thread. The foreground
//! task sleeps on a stored `Waker` until `submit` wakes it, so an idle server
//! costs no timer polling.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use gpui_kit::App;
use jade_debug_protocol::{Answer, Command, ReqId};

use super::client::Client;
use super::commands;

/// Commands waiting for the main thread; beyond this a client is told `busy`.
const MAX_QUEUED: usize = 256;

pub(super) struct Job {
    pub client: Arc<Client>,
    pub req: ReqId,
    pub command: Command,
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
            job.client.reply(
                job.req,
                Answer::Error {
                    message: "busy: command queue full".into(),
                },
            );
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

/// Starts the main-thread task that runs submitted commands. No-op unless the
/// debug server was requested.
pub(crate) fn install(cx: &mut App) {
    if !super::requested() {
        return;
    }
    cx.spawn(async move |cx| {
        loop {
            let job = NextJob.await;
            cx.update(|cx| {
                let answer = commands::execute(job.command, cx);
                job.client.reply(job.req, answer);
            });
        }
    })
    .detach();
}
