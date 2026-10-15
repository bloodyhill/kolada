//! Kolada's writing model in a process of its own: llama.cpp on the CPU at
//! low priority, taking framed requests on stdin and answering each once on
//! stdout. Nothing else is ever written to stdout.
//!
//! `kolada-ai --model <path> [--threads N] [--batch-threads N] [--ctx N]`
//! (2 threads to write, 4 to read a prompt, a context of 2048 tokens: the
//! eval's choice, lightest for the speed it needs).
//!
//! It exits 0 when stdin closes, 1 on bad arguments, 2 when the model does
//! not load. `KOLADA_AI_LOG` names a log to append to; `KOLADA_AI_TRACE` adds
//! each choice a fix weighs.

mod nai;
mod spell;

use std::collections::HashMap;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use nai::{Model, Run, note};
use kolada_assist::prompt;
use kolada_assist::queue::{Abort, Queue};
use kolada_assist::wire::{self, Kind, Reply, Request, Status};

struct Args {
    model: PathBuf,
    threads: i32,
    batch_threads: i32,
    ctx: i32,
    keep: Keep,
}

/// A fix's margins over the text as written (see `prompt::KEEP_NEAR`).
#[derive(Clone, Copy)]
struct Keep {
    near: f32,
    far: f32,
}

fn args() -> Option<Args> {
    let keep = Keep { near: prompt::KEEP_NEAR, far: prompt::KEEP_FAR };
    // A gist too long for the context is cut to fit, so 2048 is room enough.
    let mut a = Args { model: PathBuf::new(), threads: 2, batch_threads: 4, ctx: 2048, keep };
    let mut it = std::env::args_os().skip(1);
    while let Some(k) = it.next() {
        let v = it.next()?;
        let n = || v.to_str()?.parse::<i32>().ok().filter(|&n| n > 0);
        let f = || v.to_str()?.parse::<f32>().ok().filter(|f| f.is_finite());
        match k.to_str()? {
            "--model" => a.model = PathBuf::from(&v),
            "--threads" => a.threads = n()?,
            "--batch-threads" => a.batch_threads = n()?,
            "--ctx" => a.ctx = n()?,
            // For the eval's tuning.
            "--near" => a.keep.near = f()?,
            "--far" => a.keep.far = f()?,
            _ => return None,
        }
    }
    (!a.model.as_os_str().is_empty()).then_some(a)
}

/// Efficiency mode, low memory priority and a below-normal class: typing in
/// Kolada always comes first.
#[cfg(windows)]
fn lower_priority() {
    use windows::Win32::System::Threading::*;
    unsafe {
        let me = GetCurrentProcess();
        let eco = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        };
        let _ = SetProcessInformation(me, ProcessPowerThrottling, (&raw const eco).cast(), size_of_val(&eco) as u32);
        let mem = MEMORY_PRIORITY_INFORMATION { MemoryPriority: MEMORY_PRIORITY_LOW };
        let _ = SetProcessInformation(me, ProcessMemoryPriority, (&raw const mem).cast(), size_of_val(&mem) as u32);
        let _ = SetPriorityClass(me, BELOW_NORMAL_PRIORITY_CLASS);
    }
}

#[cfg(not(windows))]
fn lower_priority() {}

struct State {
    queue: Queue,
    running: Option<Request>,
    /// What became of the running job while it ran.
    abort: Abort,
    /// Taken in and not yet answered: id → note.
    waiting: HashMap<u64, u64>,
    eof: bool,
}

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    /// The running job's stop flag, read by llama.cpp between steps.
    cancel: AtomicI32,
}

fn main() {
    lower_priority();
    let Some(a) = args() else {
        eprintln!("usage: kolada-ai --model <path> [--threads N] [--batch-threads N] [--ctx N]");
        std::process::exit(1);
    };
    nai::log(std::env::var_os("KOLADA_AI_LOG").as_deref(), std::env::var_os("KOLADA_AI_TRACE").is_some());
    let t = Instant::now();
    let Some(mut model) = Model::open(&a.model, a.threads, a.batch_threads, a.ctx) else {
        note(&format!("could not load {}", a.model.display()));
        std::process::exit(2);
    };
    note(&format!("loaded {} in {} ms, threads {}/{}, ctx {}", a.model.display(), t.elapsed().as_millis(), a.threads, a.batch_threads, model.n_ctx()));
    let tpl = model.template().unwrap_or_default();
    // ChatML as built, unless the model speaks another format, or thinks
    // aloud unless told not to (SmolLM3's `/no_think`).
    let format = if tpl.contains("<|im_start|>") && !tpl.contains("no_think") { Format::ChatMl } else { Format::Native { no_think: tpl.contains("no_think") } };
    note(&format!("template of {} bytes, prompts as {format:?}", tpl.len()));

    let shared = Shared {
        state: Mutex::new(State { queue: Queue::default(), running: None, abort: Abort::No, waiting: HashMap::new(), eof: false }),
        wake: Condvar::new(),
        cancel: AtomicI32::new(0),
    };
    std::thread::scope(|s| {
        s.spawn(|| read(&shared));
        // A model trained for Kolada (tools/train) is asked as it was trained.
        let tuned = a.model.file_name().and_then(|n| n.to_str()).is_some_and(kolada_assist::models::trained);
        note(&format!("tuned: {tuned}"));
        work(&mut model, &shared, a.keep, format, tuned);
    });
}

/// How prompts reach the model: as built (ChatML), or in its own format.
#[derive(Clone, Copy, Debug)]
enum Format {
    ChatMl,
    Native { no_think: bool },
}

/// `p` in the model's own chat format; the shared opening stays shared when
/// it is still the start of the whole.
fn render(model: &Model, format: Format, p: prompt::Prompt) -> prompt::Prompt {
    let Format::Native { no_think } = format else { return p };
    native(model, no_think, &p).unwrap_or(p)
}

fn native(model: &Model, no_think: bool, p: &prompt::Prompt) -> Option<prompt::Prompt> {
    let (mut turns, fixed) = prompt::turns(p);
    let system;
    if no_think && let Some(first) = turns.first_mut().filter(|t| t.0 == "system") {
        system = format!("/no_think\n{}", first.1);
        first.1 = &system;
    }
    let text = model.chat(&turns, true)?;
    let head = model.chat(&turns[..fixed], false).filter(|h| fixed > 0 && text.starts_with(h.as_str()));
    Some(prompt::Prompt { fixed: head.map_or(0, |h| h.len()), text })
}

fn send(replies: &[Reply]) {
    let mut out = io::stdout().lock();
    for r in replies {
        if wire::write_frame(&mut out, &r.frame()).is_err() {
            return;
        }
    }
}

fn cancelled(id: u64, note: u64) -> Reply {
    Reply { id, note, status: Status::Cancelled, text: String::new(), ms: 0 }
}

/// The ids waiting in the queue, which does not list them: taken out and
/// put back in the same order.
fn queued(q: &mut Queue) -> Vec<u64> {
    let jobs: Vec<Request> = std::iter::from_fn(|| q.pop()).collect();
    let ids = jobs.iter().map(|j| j.id).collect();
    for j in jobs {
        q.push(j, None);
    }
    ids
}

/// Cancelled replies for the jobs the queue let go (replaced, or of a note
/// no longer open), so every request is answered once.
fn sweep(st: &mut State) -> Vec<Reply> {
    let mut keep = queued(&mut st.queue);
    keep.extend(st.running.as_ref().map(|j| j.id));
    let gone: Vec<u64> = st.waiting.keys().filter(|id| !keep.contains(id)).copied().collect();
    gone.into_iter().filter_map(|id| st.waiting.remove(&id).map(|note| cancelled(id, note))).collect()
}

fn read(shared: &Shared) {
    let mut input = io::stdin().lock();
    while let Ok(Some(frame)) = wire::read_frame(&mut input) {
        let Some(r) = Request::parse(&frame) else {
            note("a frame that does not parse");
            continue;
        };
        let mut st = shared.state.lock().unwrap();
        let st = &mut *st;
        if r.kind == Kind::Cancel {
            note(&format!("cancel, keeping note {}", r.note));
        } else {
            st.waiting.insert(r.id, r.note);
        }
        let abort = st.queue.push(r, st.running.as_ref());
        if abort != Abort::No {
            st.abort = st.abort.max(abort);
            shared.cancel.store(1, Ordering::SeqCst);
        }
        let replies = sweep(st);
        shared.wake.notify_one();
        send(&replies);
    }
    shared.state.lock().unwrap().eof = true;
    shared.cancel.store(1, Ordering::SeqCst);
    shared.wake.notify_one();
}

fn work(model: &mut Model, shared: &Shared, keep: Keep, format: Format, tuned: bool) {
    let mut dictionaries = spell::Dictionaries::new();
    loop {
        let job = {
            let mut st = shared.state.lock().unwrap();
            loop {
                if st.eof {
                    return;
                }
                if let Some(j) = st.queue.pop() {
                    st.running = Some(j.clone());
                    st.abort = Abort::No;
                    shared.cancel.store(0, Ordering::SeqCst);
                    break j;
                }
                st = shared.wake.wait(st).unwrap();
            }
        };
        let t = Instant::now();
        let hints = if job.kind == Kind::Fix && !tuned { dictionaries.hints(prompt::fix_language(&job), &job.text) } else { String::new() };
        if !hints.is_empty() {
            note(&format!("hints for id={}: {} words", job.id, hints.matches(" (maybe ").count()));
        }
        let run = answer(model, &job, &hints, &shared.cancel, keep, format, tuned);
        let ms = t.elapsed().as_millis().min(u32::MAX as u128) as u32;
        let s = model.last();
        let mut st = shared.state.lock().unwrap();
        st.running = None;
        let abort = std::mem::replace(&mut st.abort, Abort::No);
        shared.cancel.store(0, Ordering::SeqCst);
        let status = match (abort, run) {
            (Abort::Drop, _) | (Abort::No, Run::Cancelled) => Some((Status::Cancelled, String::new())),
            (_, Run::Done(text)) => Some((Status::Done, text)),
            (Abort::Requeue, _) => None,
            (Abort::No, Run::Long | Run::Failed) => Some((Status::Failed, String::new())),
        };
        let word = status.as_ref().map_or("requeued", |(s, _)| match s {
            Status::Done => "done",
            Status::Failed => "failed",
            Status::Cancelled => "cancelled",
        });
        note(&format!(
            "{:?} id={} note={} slot={} prompt={} cached={} out={} kept={} left={} prefill={:.1}ms decode={:.1}ms total={ms}ms {word}",
            job.kind, job.id, job.note, job.slot, s.n_prompt, s.n_cached, s.n_out, s.n_kept, s.n_left, s.prefill_ms, s.decode_ms
        ));
        let mut replies = Vec::new();
        match status {
            Some((status, text)) => {
                st.waiting.remove(&job.id);
                replies.push(Reply { id: job.id, note: job.note, status, text, ms });
            }
            None => st.queue.requeue(job),
        }
        replies.extend(sweep(&mut st));
        drop(st);
        send(&replies);
    }
}

/// Runs one job: its prompt, grammar, budget and anchor. A gist too long
/// for the context keeps the start of the note.
#[allow(clippy::too_many_arguments)]
fn answer(model: &mut Model, job: &Request, hints: &str, cancel: &AtomicI32, keep: Keep, format: Format, tuned: bool) -> Run {
    let budget = prompt::max_tokens(job.kind, model.tokens(&job.text));
    let room = model.n_ctx().saturating_sub(budget + 1);
    let mut job = job.clone();
    let mut p = render(model, format, if tuned { prompt::build_tuned(&job) } else { prompt::build_hinted(&job, hints) });
    for _ in 0..4 {
        let n = model.tokens(&p.text);
        if n <= room {
            // A model trained for fixes keeps to the text on its own, and
            // an anchor only breaks its fixes up (`Estoes` stays `Estoes`);
            // the guard judges what it answers. A margin below zero: none
            // for any model.
            if let Some(a) = prompt::anchor(&job).filter(|_| !tuned && keep.near >= 0.0) {
                model.anchor(&a, keep.near, keep.far);
            }
            return model.run(&p.text, p.fixed, prompt::grammar(&job).as_deref(), budget, cancel);
        }
        if job.kind != Kind::Gist {
            break;
        }
        let over = n - room;
        let cut = job.text.len().saturating_sub(over * job.text.len() / model.tokens(&job.text).max(1) + 64);
        job.text.truncate(job.text.floor_char_boundary(cut));
        p = render(model, format, prompt::build(&job));
    }
    Run::Failed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(kind: Kind, id: u64, note: u64, slot: u64) -> Request {
        Request { kind, id, note, slot, context: String::new(), text: String::new() }
    }

    fn state() -> State {
        State { queue: Queue::default(), running: None, abort: Abort::No, waiting: HashMap::new(), eof: false }
    }

    fn take(st: &mut State, r: Request) -> Abort {
        if r.kind != Kind::Cancel {
            st.waiting.insert(r.id, r.note);
        }
        st.queue.push(r, st.running.as_ref())
    }

    #[test]
    fn listing_the_queue_leaves_it_as_it_was() {
        let mut st = state();
        for (kind, id, slot) in [(Kind::Gist, 1, 0), (Kind::Fix, 2, 5), (Kind::Shape, 3, 5), (Kind::Fix, 4, 9)] {
            take(&mut st, job(kind, id, 1, slot));
        }
        assert_eq!(queued(&mut st.queue), [2, 4, 3, 1]);
        assert_eq!(queued(&mut st.queue), [2, 4, 3, 1]);
        let order: Vec<u64> = std::iter::from_fn(|| st.queue.pop()).map(|j| j.id).collect();
        assert_eq!(order, [2, 4, 3, 1]);
    }

    #[test]
    fn every_job_let_go_is_answered_cancelled_once() {
        let mut st = state();
        take(&mut st, job(Kind::Fix, 1, 1, 5));
        take(&mut st, job(Kind::Gist, 2, 1, 0));
        st.running = st.queue.pop();
        assert_eq!(take(&mut st, job(Kind::Fix, 3, 1, 5)), Abort::Drop, "a newer fix for the running one's place");
        assert!(sweep(&mut st).is_empty(), "the running one answers for itself");
        take(&mut st, job(Kind::Fix, 4, 1, 5));
        let gone = sweep(&mut st);
        assert_eq!(gone.iter().map(|r| (r.id, r.status)).collect::<Vec<_>>(), [(3, Status::Cancelled)]);
        assert!(sweep(&mut st).is_empty(), "once");

        assert_eq!(take(&mut st, job(Kind::Cancel, 0, 2, 0)), Abort::Drop);
        let mut gone: Vec<u64> = sweep(&mut st).iter().map(|r| r.id).collect();
        gone.sort();
        assert_eq!(gone, [2, 4], "the old note's queued jobs");
        take(&mut st, job(Kind::Fix, 5, 1, 7));
        assert_eq!(sweep(&mut st).iter().map(|r| r.id).collect::<Vec<_>>(), [5], "a late job of a closed note");
        assert_eq!(st.waiting.keys().copied().collect::<Vec<_>>(), [1], "only the running one is still owed");
    }
}
