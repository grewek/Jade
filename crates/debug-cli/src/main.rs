//! `jade-debug`: drive and observe a running Jade over its debug socket.
//! Run `jade-debug help`.

mod args;
mod client;
mod fuzz;
mod launch;
mod render;

use std::io::BufRead;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use args::{Action, parse_action, parse_cli, split_words};
use client::Connection;
use jade_debug_protocol::{Answer, Command, Event, Message, Query, Topic};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("jade-debug: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let words: Vec<String> = std::env::args().skip(1).collect();
    let cli = parse_cli(&words, std::env::var("JADE_DEBUG_SOCKET").ok())?;
    match cli.action {
        Action::Help => {
            print!("{}", args::HELP);
            Ok(())
        }
        Action::Launch(launch_args) => launch::run(&cli.socket, launch_args),
        Action::Script => script(&mut Connection::open(&cli.socket)?),
        action => execute(&mut Connection::open(&cli.socket)?, action),
    }
}

fn print_events(events: &[Event]) {
    for event in events {
        println!("{}", render::event_line(event));
    }
}

fn execute(conn: &mut Connection, action: Action) -> Result<(), String> {
    match action {
        Action::Send(command) => send(conn, command),
        Action::Capture { command, out } => capture(conn, command, &out),
        Action::Frames { raw: false } => send(
            conn,
            Command::Query {
                what: Query::FrameTimes,
            },
        ),
        Action::Frames { raw: true } => {
            let (answer, events) = conn.call(Command::Query {
                what: Query::FrameTimes,
            })?;
            print_events(&events);
            let jade_debug_protocol::Answer::FrameTimes { frames } = answer else {
                return render::answer_text(&answer).map(|text| println!("{text}"));
            };
            frames.iter().for_each(|f| println!("{f:?}"));
            Ok(())
        }
        Action::Sleep(ms) => pump_for(conn, Duration::from_millis(ms)),
        Action::Watch { topic, seconds } => watch(conn, topic, seconds),
        Action::Script | Action::Launch(_) | Action::Help => {
            Err("that command cannot run inside a script".into())
        }
    }
}

fn send(conn: &mut Connection, command: Command) -> Result<(), String> {
    let (answer, events) = conn.call(command)?;
    print_events(&events);
    println!("{}", render::answer_text(&answer)?);
    Ok(())
}

/// Sends a `Capture`, reads the PNG that follows and writes it to `out`.
fn capture(conn: &mut Connection, command: Command, out: &std::path::Path) -> Result<(), String> {
    let (answer, events) = conn.call(command)?;
    print_events(&events);
    let Answer::Capture(info) = answer else {
        return render::answer_text(&answer).map(|text| println!("{text}"));
    };
    let (png, events) = conn.read_chunks(info.chunks)?;
    print_events(&events);
    if png.len() != info.bytes {
        return Err(format!("expected {} bytes, got {}", info.bytes, png.len()));
    }
    std::fs::write(out, &png).map_err(|e| format!("{}: {e}", out.display()))?;
    println!(
        "wrote {} ({}x{} px, {} bytes) of the region at {:.1},{:.1} {:.1}x{:.1}",
        out.display(),
        info.width,
        info.height,
        info.bytes,
        info.bounds.x,
        info.bounds.y,
        info.bounds.w,
        info.bounds.h
    );
    Ok(())
}

/// Waits `duration`, printing any events that arrive meanwhile.
fn pump_for(conn: &mut Connection, duration: Duration) -> Result<(), String> {
    let deadline = Instant::now() + duration;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(());
        }
        if let Some(Message::Event { event }) = conn.next_message(left)? {
            println!("{}", render::event_line(&event));
        }
    }
}

fn watch(conn: &mut Connection, topic: Topic, seconds: u64) -> Result<(), String> {
    send(conn, Command::Subscribe { topic })?;
    pump_for(conn, Duration::from_secs(seconds))
}

/// One command per stdin line; stops at the first failure.
fn script(conn: &mut Connection) -> Result<(), String> {
    for (index, line) in std::io::stdin().lock().lines().enumerate() {
        let line = line.map_err(|e| e.to_string())?;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = |e: String| format!("script line {}: {e}", index + 1);
        let words = split_words(line).map_err(at)?;
        let action = parse_action(&words).map_err(at)?;
        execute(conn, action).map_err(at)?;
    }
    Ok(())
}
