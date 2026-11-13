//! The playground's JavaScript engine: QuickJS (via `rquickjs`) in a fresh runtime per run.
//!
//! User code gets no DOM, network, timers, modules or Tauri globals: only the ECMAScript
//! built-ins, `console` and a read-only `note(title)`. A run is bounded by a wall-clock
//! deadline and a Stop flag (both through the interrupt handler, which QuickJS polls while
//! executing), a memory limit and a stack limit. Output is captured (not streamed) and
//! capped; the last value and the run's error are always reported.
//!
//! Nothing here may panic: release builds abort on panic, and the engine callbacks run
//! inside C frames. Every fallible step returns a `Result` and is mapped to a [`RunError`].
//!
//! A run happens in three phases so that pending jobs can be run through
//! `Runtime::execute_pending_job`, which (unlike `Promise::finish`) hands back the exception
//! of a job that threw: (1) inside `Context::with`, install the prelude and evaluate the
//! script; (2) outside it, run jobs until the queue is empty; (3) inside it again, read the
//! eval promise, format the last value and collect what went wrong. JS values that cross
//! phases are `Persistent` and all of them are dropped before the runtime.
use rquickjs::context::EvalOptions;
use rquickjs::promise::PromiseState;
use rquickjs::{Context, Ctx, Exception, Function, Object, Persistent, Promise, Runtime, Value};
use serde::Serialize;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const PRELUDE: &str = include_str!("prelude.js");
const PRELUDE_FILE: &str = "badger_prelude";
/// The file name QuickJS gives user code; stack frames in it carry `eval_script:LINE:COL`.
const SCRIPT_FILE: &str = "eval_script";
const NEVER_SETTLES: &str = "Script is waiting on a promise that never settles";
const UNSHOWABLE_VALUE: &str = "[the value could not be shown]";

/// Limits for one run.
#[derive(Debug, Clone)]
pub struct RunOptions {
    pub timeout: Duration,
    /// QuickJS heap limit in bytes.
    pub memory_limit: usize,
    /// QuickJS stack limit in bytes.
    pub max_stack: usize,
    /// Captured console output, in UTF-8 bytes.
    pub output_bytes: usize,
    /// Captured console output, in lines (a logged value with newlines counts each line).
    pub output_lines: usize,
}

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            timeout: Duration::from_secs(10),
            memory_limit: 256 * 1024 * 1024,
            max_stack: 1024 * 1024,
            output_bytes: 1024 * 1024,
            output_lines: 10_000,
        }
    }
}

/// What `note(title)` reads. The lookup returns `Ok(None)` for an unknown title.
#[derive(Debug, Clone, PartialEq)]
pub enum NoteValue {
    /// A JSON note's body; the script receives it parsed.
    Json(String),
    /// A markdown note's body, as a string.
    Markdown(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Log,
    Info,
    Warn,
    Error,
    Debug,
}

impl Level {
    fn parse(level: &str) -> Level {
        match level {
            "info" => Level::Info,
            "warn" => Level::Warn,
            "error" => Level::Error,
            "debug" => Level::Debug,
            _ => Level::Log,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OutputLine {
    pub level: Level,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorKind {
    /// An uncaught exception or rejection from the script (also: a promise that never settles).
    User,
    Timeout,
    Stopped,
    Oom,
    Stack,
    Syntax,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunError {
    pub kind: ErrorKind,
    pub message: String,
    /// 1-based position in the script, when the error points into it.
    pub line: Option<u32>,
    pub col: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RunOutcome {
    pub lines: Vec<OutputLine>,
    /// The formatted completion value; None when it was `undefined` (or the run failed).
    pub value: Option<String>,
    pub error: Option<RunError>,
    /// Output hit the cap; later console output was dropped.
    pub truncated: bool,
    pub duration_ms: u64,
}

/// Why the interrupt handler stopped the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Interrupt {
    Timeout,
    Stopped,
}

/// Console output collected by the host `emit` hook, with the caps applied.
struct Sink {
    lines: Vec<OutputLine>,
    bytes: usize,
    line_count: usize,
    truncated: bool,
    max_bytes: usize,
    max_lines: usize,
}

impl Sink {
    fn new(opts: &RunOptions) -> Self {
        Sink {
            lines: Vec::new(),
            bytes: 0,
            line_count: 0,
            truncated: false,
            max_bytes: opts.output_bytes,
            max_lines: opts.output_lines,
        }
    }

    /// Adds a line; past the cap it keeps the part that fits, marks the output truncated
    /// and returns false (the prelude then stops formatting console calls).
    fn push(&mut self, level: Level, text: String) -> bool {
        if self.truncated {
            return false;
        }
        let lines = text.matches('\n').count() + 1;
        if self.bytes + text.len() <= self.max_bytes && self.line_count + lines <= self.max_lines {
            self.bytes += text.len();
            self.line_count += lines;
            self.lines.push(OutputLine { level, text });
            return true;
        }
        self.truncated = true;
        let room_bytes = self.max_bytes.saturating_sub(self.bytes);
        let room_lines = self.max_lines.saturating_sub(self.line_count);
        if room_bytes == 0 || room_lines == 0 {
            return false;
        }
        // Up to `room_lines` lines (newline excluded) and `room_bytes` bytes.
        let mut end = text
            .match_indices('\n')
            .nth(room_lines - 1)
            .map_or(text.len(), |(i, _)| i);
        end = end.min(room_bytes);
        while end > 0 && !text.is_char_boundary(end) {
            end -= 1;
        }
        if end > 0 {
            let part = text[..end].to_string();
            self.bytes += part.len();
            self.line_count += part.matches('\n').count() + 1;
            self.lines.push(OutputLine { level, text: part });
        }
        false
    }
}

/// An unhandled rejection: the promise (to match a later "handled") and its reason.
type Tracked = Vec<(Persistent<Value<'static>>, Persistent<Value<'static>>)>;

/// The prelude's hooks plus the user code's eval result, carried from phase 1 to 3.
struct Started {
    format: Persistent<Function<'static>>,
    describe: Persistent<Function<'static>>,
    eval: Result<Persistent<Promise<'static>>, Persistent<Value<'static>>>,
}

/// An exception as the user sees it: `Name: message` (or the formatted value) and stack.
struct Described {
    text: String,
    stack: Option<String>,
    is_null: bool,
}

/// Runs `code` once in a fresh QuickJS runtime and reports its output, last value and
/// error. `cancel` stops it (checked by the interrupt handler); the caller resets it.
/// `note_lookup` serves `note(title)` (`Ok(None)`: no such note; `Err`: thrown as is).
pub fn run<F>(code: &str, opts: &RunOptions, cancel: Arc<AtomicBool>, note_lookup: F) -> RunOutcome
where
    F: Fn(&str) -> Result<Option<NoteValue>, String> + 'static,
{
    let started_at = Instant::now();
    let sink = Rc::new(RefCell::new(Sink::new(opts)));
    let (value, error) = match Runtime::new() {
        Ok(rt) => match Context::full(&rt) {
            Ok(ctx) => {
                let result = run_in(&rt, &ctx, code, opts, cancel, note_lookup, &sink);
                // Drop every engine callback (and the JS values they hold) while the
                // runtime is alive; the runtime itself goes last.
                rt.set_host_promise_rejection_tracker(None);
                rt.set_interrupt_handler(None);
                drop(ctx);
                result
            }
            Err(_) => (None, Some(engine_failed())),
        },
        Err(_) => (None, Some(engine_failed())),
    };
    let (lines, truncated) = match Rc::try_unwrap(sink) {
        Ok(cell) => {
            let s = cell.into_inner();
            (s.lines, s.truncated)
        }
        // A host function still holds the sink (it can't: the context is gone); be safe.
        Err(shared) => match shared.try_borrow_mut() {
            Ok(mut s) => (std::mem::take(&mut s.lines), s.truncated),
            Err(_) => (Vec::new(), true),
        },
    };
    RunOutcome {
        lines,
        value,
        error,
        truncated,
        duration_ms: u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX),
    }
}

fn engine_failed() -> RunError {
    RunError {
        kind: ErrorKind::User,
        message: "Could not start the script engine".to_string(),
        line: None,
        col: None,
    }
}

fn run_in<F>(
    rt: &Runtime,
    ctx: &Context,
    code: &str,
    opts: &RunOptions,
    cancel: Arc<AtomicBool>,
    note_lookup: F,
    sink: &Rc<RefCell<Sink>>,
) -> (Option<String>, Option<RunError>)
where
    F: Fn(&str) -> Result<Option<NoteValue>, String> + 'static,
{
    let deadline = Instant::now() + opts.timeout;
    rt.set_memory_limit(opts.memory_limit);
    rt.set_max_stack_size(opts.max_stack);

    let fired: Rc<Cell<Option<Interrupt>>> = Rc::new(Cell::new(None));
    {
        let fired = fired.clone();
        let cancel = cancel.clone();
        rt.set_interrupt_handler(Some(Box::new(move || {
            if fired.get().is_some() {
                return true;
            }
            let reason = if cancel.load(Ordering::SeqCst) {
                Interrupt::Stopped
            } else if Instant::now() >= deadline {
                Interrupt::Timeout
            } else {
                return false;
            };
            fired.set(Some(reason));
            true
        })));
    }

    // Unhandled rejections, removed again when a handler is attached later.
    let tracked: Rc<RefCell<Tracked>> = Rc::new(RefCell::new(Vec::new()));
    {
        let tracked = tracked.clone();
        rt.set_host_promise_rejection_tracker(Some(Box::new(
            move |ctx: Ctx<'_>, promise: Value<'_>, reason: Value<'_>, is_handled: bool| {
                let Ok(mut list) = tracked.try_borrow_mut() else {
                    return;
                };
                if is_handled {
                    list.retain(|(p, _)| {
                        p.clone()
                            .restore(&ctx)
                            .map_or(true, |p: Value<'_>| p != promise)
                    });
                } else {
                    list.push((
                        Persistent::save(&ctx, promise),
                        Persistent::save(&ctx, reason),
                    ));
                }
            },
        )));
    }

    // Heap usage can't be read while JS runs (the runtime is borrowed inside `Context::with`
    // and the engine callbacks), so it is sampled between phases and after each failed job.
    // Memory freed while an exception unwinds is gone by then: a `null` out-of-memory
    // exception thrown deep in a function at a tiny limit can read as `Uncaught null`.
    let mut peak_memory: i64 = 0;
    let mut note_memory = |rt: &Runtime| {
        peak_memory = peak_memory.max(rt.memory_usage().malloc_size);
    };

    // ── Phase 1: prelude and eval ──
    let started = ctx.with(|ctx| start(&ctx, code, opts, sink, note_lookup));
    note_memory(rt);

    // ── Phase 2: run jobs until the queue is empty ──
    let mut job_errors: Vec<Persistent<Value<'static>>> = Vec::new();
    if started.is_ok() {
        loop {
            if fired.get().is_some() {
                break;
            }
            if cancel.load(Ordering::SeqCst) {
                fired.set(Some(Interrupt::Stopped));
                break;
            }
            if Instant::now() >= deadline {
                fired.set(Some(Interrupt::Timeout));
                break;
            }
            match rt.execute_pending_job() {
                Ok(true) => {}
                Ok(false) => break,
                Err(job) => {
                    note_memory(rt);
                    if fired.get().is_some() {
                        break;
                    }
                    job.0.with(|ctx| {
                        let exception = ctx.catch();
                        job_errors.push(Persistent::save(&ctx, exception));
                    });
                }
            }
        }
    }
    note_memory(rt);
    let at_memory_limit = peak_memory >= limit_threshold(opts.memory_limit);

    // ── Phase 3: results ──
    let result = ctx.with(|ctx| {
        let started = match started {
            Ok(s) => s,
            // A Stop or time-out that lands while the prelude is set up fails the setup;
            // report the interrupt, not the setup.
            Err(e) => return (None, Some(fired.get().map_or(e, |r| interrupted(r, opts)))),
        };
        finish(
            &ctx,
            started,
            job_errors,
            &tracked,
            &fired,
            at_memory_limit,
            opts,
            sink,
        )
    });
    // Anything still tracked (an interrupted phase 3) goes now, before the runtime.
    if let Ok(mut list) = tracked.try_borrow_mut() {
        list.clear();
    }
    result
}

/// 7/8 of the limit: a `null` exception at that usage is QuickJS failing to allocate the
/// "out of memory" error itself.
fn limit_threshold(limit: usize) -> i64 {
    i64::try_from(limit - limit / 8).unwrap_or(i64::MAX)
}

/// Phase 1: installs the prelude and host hooks, then evaluates the script (as a global
/// async script, so top-level `await` works). `Err` only when the engine can't be set up or
/// the source can't be handed to it at all.
fn start<F>(
    ctx: &Ctx<'_>,
    code: &str,
    opts: &RunOptions,
    sink: &Rc<RefCell<Sink>>,
    note_lookup: F,
) -> Result<Started, RunError>
where
    F: Fn(&str) -> Result<Option<NoteValue>, String> + 'static,
{
    let (format, describe) =
        install_prelude(ctx, opts, sink, note_lookup).map_err(|_| engine_failed())?;
    let mut options = EvalOptions::default();
    options.global = true;
    options.promise = true;
    options.strict = false;
    options.filename = Some(SCRIPT_FILE.to_string());
    let eval = match ctx.eval_with_options::<Promise, _>(code, options) {
        Ok(promise) => Ok(Persistent::save(ctx, promise)),
        Err(rquickjs::Error::Exception) => Err(Persistent::save(ctx, ctx.catch())),
        Err(rquickjs::Error::InvalidString(_)) => {
            return Err(RunError {
                kind: ErrorKind::Syntax,
                message: "SyntaxError: the script contains a NUL character".to_string(),
                line: None,
                col: None,
            })
        }
        Err(e) => {
            return Err(RunError {
                kind: ErrorKind::User,
                message: format!("Could not run the script: {e}"),
                line: None,
                col: None,
            })
        }
    };
    Ok(Started {
        format: Persistent::save(ctx, format),
        describe: Persistent::save(ctx, describe),
        eval,
    })
}

/// Evaluates the prelude and calls it with the host hooks; returns its `format` and
/// `describeError` functions.
fn install_prelude<'js, F>(
    ctx: &Ctx<'js>,
    opts: &RunOptions,
    sink: &Rc<RefCell<Sink>>,
    note_lookup: F,
) -> rquickjs::Result<(Function<'js>, Function<'js>)>
where
    F: Fn(&str) -> Result<Option<NoteValue>, String> + 'static,
{
    let host = Object::new(ctx.clone())?;
    let emit_sink = sink.clone();
    let emit = Function::new(ctx.clone(), move |level: String, text: String| -> bool {
        match emit_sink.try_borrow_mut() {
            Ok(mut s) => s.push(Level::parse(&level), text),
            Err(_) => false,
        }
    })?;
    host.set("emit", emit)?;
    let note = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, title: String| -> rquickjs::Result<String> {
            match note_lookup(&title) {
                Ok(Some(NoteValue::Json(body))) => Ok(format!("j{body}")),
                Ok(Some(NoteValue::Markdown(body))) => Ok(format!("m{body}")),
                Ok(None) => Err(Exception::throw_message(
                    &ctx,
                    &format!("Note \"{title}\" not found"),
                )),
                Err(message) => Err(Exception::throw_message(&ctx, &message)),
            }
        },
    )?;
    host.set("note", note)?;

    let mut options = EvalOptions::default();
    options.filename = Some(PRELUDE_FILE.to_string());
    let prelude: Function = ctx.eval_with_options(PRELUDE, options)?;
    // Characters, not bytes: the prelude measures JS strings in UTF-16 units.
    let max_chars = f64::from(u32::try_from(opts.output_bytes).unwrap_or(u32::MAX));
    let hooks: Object = prelude.call((host, max_chars))?;
    Ok((hooks.get("format")?, hooks.get("describeError")?))
}

/// Phase 3: the run's value and error.
#[allow(clippy::too_many_arguments)]
fn finish<'js>(
    ctx: &Ctx<'js>,
    started: Started,
    job_errors: Vec<Persistent<Value<'static>>>,
    tracked: &Rc<RefCell<Tracked>>,
    fired: &Rc<Cell<Option<Interrupt>>>,
    at_memory_limit: bool,
    opts: &RunOptions,
    sink: &Rc<RefCell<Sink>>,
) -> (Option<String>, Option<RunError>) {
    let rejections = match tracked.try_borrow_mut() {
        Ok(mut list) => std::mem::take(&mut *list),
        Err(_) => Vec::new(),
    };
    if let Some(reason) = fired.get() {
        return (None, Some(interrupted(reason, opts)));
    }
    let (Ok(format), Ok(describe)) = (started.format.restore(ctx), started.describe.restore(ctx))
    else {
        return (None, Some(engine_failed()));
    };
    let describe_value = |v: Value<'js>| describe_exception(&describe, v);

    let mut value = None;
    let mut error = None;
    let mut eval_promise: Option<Value<'js>> = None;
    match started.eval {
        Err(exception) => {
            // The script didn't compile (or threw before it could start).
            if let Ok(exception) = exception.restore(ctx) {
                let d = describe_value(exception);
                let syntax = d.text.starts_with("SyntaxError");
                error = Some(classify(d, at_memory_limit, opts, "Uncaught ", syntax));
            }
        }
        Ok(promise) => match promise.restore(ctx) {
            Err(_) => error = Some(engine_failed()),
            Ok(promise) => {
                eval_promise = Some(promise.clone().into_value());
                match promise.state() {
                    PromiseState::Resolved => {
                        let completion = promise
                            .result::<Object>()
                            .and_then(Result::ok)
                            .and_then(|o| o.get::<_, Value>("value").ok());
                        if let Some(v) = completion.filter(|v| !v.is_undefined()) {
                            // The prelude returns well-formed text; if formatting itself
                            // fails (out of memory), say so rather than drop the value.
                            value = Some(
                                format
                                    .call::<_, String>((v,))
                                    .unwrap_or_else(|_| UNSHOWABLE_VALUE.to_string()),
                            );
                        }
                    }
                    PromiseState::Rejected => {
                        let reason = promise
                            .result::<Value>()
                            .and_then(Result::err)
                            .map(|_| ctx.catch());
                        if let Some(reason) = reason {
                            let d = describe_value(reason);
                            error = Some(classify(d, at_memory_limit, opts, "Uncaught ", false));
                        }
                    }
                    PromiseState::Pending => {
                        error = Some(RunError {
                            kind: ErrorKind::User,
                            message: NEVER_SETTLES.to_string(),
                            line: None,
                            col: None,
                        });
                    }
                }
            }
        },
    }

    // Uncaught errors in jobs, then rejections still unhandled now (not the eval promise's
    // own, which is the run's error above). The first becomes the run's error when it has
    // none; the rest are logged.
    let mut late: Vec<RunError> = Vec::new();
    for exception in job_errors {
        if let Ok(v) = exception.restore(ctx) {
            late.push(classify(
                describe_value(v),
                at_memory_limit,
                opts,
                "Uncaught ",
                false,
            ));
        }
    }
    for (promise, reason) in rejections {
        let (Ok(promise), Ok(reason)) = (promise.restore(ctx), reason.restore(ctx)) else {
            continue;
        };
        if eval_promise.as_ref() == Some(&promise) {
            continue;
        }
        late.push(classify(
            describe_value(reason),
            at_memory_limit,
            opts,
            "Uncaught (in promise) ",
            false,
        ));
    }
    // An interrupt while formatting (the value or an error) wins over everything.
    if let Some(reason) = fired.get() {
        return (None, Some(interrupted(reason, opts)));
    }
    for e in late {
        if error.is_none() {
            error = Some(e);
        } else if let Ok(mut s) = sink.try_borrow_mut() {
            s.push(Level::Error, e.message);
        }
    }
    (value, error)
}

fn interrupted(reason: Interrupt, opts: &RunOptions) -> RunError {
    let message = match reason {
        Interrupt::Stopped => "Stopped".to_string(),
        Interrupt::Timeout => format!("Script timed out after {}", seconds(opts.timeout)),
    };
    let kind = match reason {
        Interrupt::Stopped => ErrorKind::Stopped,
        Interrupt::Timeout => ErrorKind::Timeout,
    };
    RunError {
        kind,
        message,
        line: None,
        col: None,
    }
}

/// `10 s`, `0.25 s`.
fn seconds(d: Duration) -> String {
    let ms = d.as_millis();
    if ms % 1000 == 0 {
        format!("{} s", ms / 1000)
    } else {
        format!("{} s", d.as_secs_f64())
    }
}

fn describe_exception<'js>(describe: &Function<'js>, exception: Value<'js>) -> Described {
    let is_null = exception.is_null();
    match describe.call::<_, Vec<Value>>((exception,)) {
        Ok(parts) => {
            let text = parts
                .first()
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_string().ok())
                .unwrap_or_else(|| "exception".to_string());
            let stack = parts
                .get(1)
                .and_then(|v| v.as_string())
                .and_then(|s| s.to_string().ok());
            Described {
                text,
                stack,
                is_null,
            }
        }
        // Out of memory (or interrupted) while describing it.
        Err(_) => Described {
            text: if is_null { "null" } else { "exception" }.to_string(),
            stack: None,
            is_null,
        },
    }
}

/// Maps an exception to a [`RunError`]. Out of memory is the engine's own `InternalError:
/// out of memory`, or a `null` exception at the memory limit (QuickJS could not allocate
/// the error object); a plain `throw null` stays a user error.
///
/// The `InternalError` is taken at any memory use: one huge request (`'x'.repeat(5e7)`)
/// fails while usage is still low, and QuickJS's error is indistinguishable from one the
/// script builds itself (same class, message and stack shape), so a script that throws
/// `new InternalError("out of memory")` also reads as out of memory.
fn classify(
    d: Described,
    at_memory_limit: bool,
    opts: &RunOptions,
    prefix: &str,
    syntax: bool,
) -> RunError {
    let oom_error = d.text == "InternalError: out of memory";
    let null_oom = d.is_null && at_memory_limit;
    #[cfg(test)]
    if null_oom {
        tests::NULL_OOMS.with(|n| n.set(n.get() + 1));
    }
    if oom_error || null_oom {
        return RunError {
            kind: ErrorKind::Oom,
            message: format!(
                "Out of memory (the limit is {} MB)",
                opts.memory_limit / (1024 * 1024)
            ),
            line: None,
            col: None,
        };
    }
    let kind = if syntax {
        ErrorKind::Syntax
    } else if d
        .text
        .starts_with("RangeError: Maximum call stack size exceeded")
    {
        ErrorKind::Stack
    } else {
        ErrorKind::User
    };
    let (line, col) = d
        .stack
        .as_deref()
        .and_then(script_position)
        .map_or((None, None), |(l, c)| (Some(l), Some(c)));
    RunError {
        kind,
        message: format!("{prefix}{}", d.text),
        line,
        col,
    }
}

/// The first `eval_script:LINE:COL` in a stack trace: the innermost frame in user code.
pub(crate) fn script_position(stack: &str) -> Option<(u32, u32)> {
    /// A leading run of ASCII digits as a number, and the text after it.
    fn number(s: &str) -> Option<(u32, &str)> {
        let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        let (digits, rest) = s.split_at(end);
        digits.parse().ok().map(|n| (n, rest))
    }
    stack.split(SCRIPT_FILE).skip(1).find_map(|frame| {
        let (line, rest) = number(frame.strip_prefix(':')?)?;
        let (col, _) = number(rest.strip_prefix(':')?)?;
        Some((line, col))
    })
}

#[cfg(test)]
mod tests;
