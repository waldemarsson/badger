use super::*;
use std::collections::HashMap;

thread_local! {
    /// `null` exceptions classify() took for out of memory on this thread.
    pub(super) static NULL_OOMS: Cell<u32> = const { Cell::new(0) };
}

fn opts() -> RunOptions {
    RunOptions {
        timeout: Duration::from_millis(1500),
        ..RunOptions::default()
    }
}

fn notes() -> HashMap<String, NoteValue> {
    let mut m = HashMap::new();
    m.insert(
        "data".to_string(),
        NoteValue::Json(r#"{"items":[1,2,3],"name":"Åsa"}"#.to_string()),
    );
    m.insert(
        "plan".to_string(),
        NoteValue::Markdown("# Plan\n- [ ] ship".to_string()),
    );
    m.insert("broken".to_string(), NoteValue::Json("{oops".to_string()));
    m
}

fn run_with(code: &str, opts: &RunOptions) -> RunOutcome {
    let notes = notes();
    run(code, opts, Arc::new(AtomicBool::new(false)), move |t| {
        Ok(notes.get(&t.to_lowercase()).cloned())
    })
}

fn run_ok(code: &str) -> RunOutcome {
    run_with(code, &opts())
}

fn texts(o: &RunOutcome) -> Vec<&str> {
    o.lines.iter().map(|l| l.text.as_str()).collect()
}

fn err(o: &RunOutcome) -> &RunError {
    o.error.as_ref().expect("expected an error")
}

fn value(code: &str) -> String {
    let o = run_ok(code);
    assert_eq!(o.error, None, "{code}");
    o.value.expect("expected a value")
}

// ── AC3: console capture and the last value ──

#[test]
fn console_lines_keep_their_level_and_join_args() {
    let o = run_ok(
        "console.log('a', 1, true); console.info('i'); console.warn('w'); \
         console.error('e'); console.debug('d'); console.log(); console.log('Åsa 🦡')",
    );
    assert_eq!(o.error, None);
    let got: Vec<(Level, &str)> = o.lines.iter().map(|l| (l.level, l.text.as_str())).collect();
    assert_eq!(
        got,
        [
            (Level::Log, "a 1 true"),
            (Level::Info, "i"),
            (Level::Warn, "w"),
            (Level::Error, "e"),
            (Level::Debug, "d"),
            (Level::Log, ""),
            (Level::Log, "Åsa 🦡"),
        ]
    );
    assert_eq!(o.value, None, "console.log returns undefined");
    assert!(!o.truncated);
}

#[test]
fn last_value_is_reported_and_undefined_is_omitted() {
    assert_eq!(value("1 + 2"), "3");
    assert_eq!(value("await Promise.resolve(41) + 1"), "42");
    assert_eq!(value("const x = await 5; x * 2"), "10");
    assert_eq!(value("'text'"), "text");
    let o = run_ok("undefined");
    assert_eq!((o.value, o.error), (None, None));
    let o = run_ok("let a = 1;");
    assert_eq!((o.value, o.error), (None, None));
}

#[test]
fn empty_script_succeeds_with_no_output() {
    for code in ["", "   \n// just a comment\n"] {
        let o = run_ok(code);
        assert_eq!(o.error, None);
        assert_eq!(o.value, None);
        assert!(o.lines.is_empty());
    }
}

#[test]
fn sloppy_mode_and_no_host_globals() {
    assert_eq!(value("x = 4; x"), "4");
    assert_eq!(
        value(
            "[typeof setTimeout, typeof fetch, typeof window, typeof __TAURI_INTERNALS__, \
              typeof require, typeof process].join()"
        ),
        "undefined,undefined,undefined,undefined,undefined,undefined"
    );
    // The prelude's host object is not reachable.
    assert_eq!(value("typeof host + typeof emit"), "undefinedundefined");
}

// ── AC8, AC14: formatting ──

#[test]
fn formatter_handles_nesting_cycles_collections_and_types() {
    assert_eq!(
        value("({a: 1, b: 'x', c: [1, 'two', null]})"),
        "{ a: 1, b: \"x\", c: [1, \"two\", null] }"
    );
    assert_eq!(
        value("const o = {name: 'o'}; o.self = o; o"),
        "{ name: \"o\", self: [Circular] }"
    );
    assert_eq!(value("const a = [1]; a.push(a); a"), "[1, [Circular]]");
    assert_eq!(
        value("new Map([['k', {v: 1}], [2, 'b']])"),
        "Map(2) { \"k\" => { v: 1 }, 2 => \"b\" }"
    );
    assert_eq!(value("new Set([1, 'a'])"), "Set(2) { 1, \"a\" }");
    assert_eq!(value("new Map()"), "Map(0) {}");
    assert_eq!(value("function add(a, b) {}; add"), "[Function: add]");
    assert_eq!(value("(() => {})"), "[Function (anonymous)]");
    assert_eq!(value("class Foo {}; Foo"), "[class Foo]");
    assert_eq!(value("(async function go() {})"), "[AsyncFunction: go]");
    assert_eq!(
        value("class P { constructor() { this.x = 1 } }; new P()"),
        "P { x: 1 }"
    );
    assert_eq!(value("Object.create(null)"), "[Object: null prototype] {}");
    assert_eq!(
        value("[-0, 1n, Symbol('s'), undefined, NaN]"),
        "[-0, 1n, Symbol(s), undefined, NaN]"
    );
    assert_eq!(value("new Date(0)"), "1970-01-01T00:00:00.000Z");
    assert_eq!(value("/a+b/gi"), "/a+b/gi");
    assert_eq!(value("new TypeError('bad')"), "[TypeError: bad]");
    assert_eq!(
        value("({'a-b': 1, [Symbol('k')]: 2})"),
        "{ \"a-b\": 1, [Symbol(k)]: 2 }"
    );
    assert_eq!(value("[1, , 3]"), "[1, <empty>, 3]");
    assert_eq!(value("({get g() { return 1 }})"), "{ g: [Getter] }");
    assert_eq!(
        value("'line\\n\\u0007'"),
        "line\n\u{7}",
        "top-level strings stay raw"
    );
    assert_eq!(
        value("['line\\n']"),
        "[\"line\\n\"]",
        "nested strings are quoted"
    );
}

#[test]
fn formatter_pretty_prints_long_values_across_lines() {
    let v = value("({alpha: 'aaaaaaaaaaaaaaaa', beta: 'bbbbbbbbbbbbbbbbbbb', gamma: [1, 2, 3], delta: {x: 1}})");
    assert_eq!(
        v,
        "{\n  alpha: \"aaaaaaaaaaaaaaaa\",\n  beta: \"bbbbbbbbbbbbbbbbbbb\",\n  gamma: [1, 2, 3],\n  delta: { x: 1 }\n}"
    );
    let deep =
        value("let o = {}; let c = o; for (let i = 0; i < 20; i++) { c.n = {}; c = c.n }; o");
    assert!(deep.contains("[Object]"), "depth is capped: {deep}");
}

#[test]
fn formatter_survives_hostile_prototypes() {
    let o = run_ok(
        "Array.prototype.map = null; Array.prototype.join = null; Array.prototype.push = null;
         Object.keys = null; JSON.stringify = null; String.prototype.slice = null;
         Object.defineProperty(Array.prototype, '0', { set() { throw new Error('trap') }, configurable: true });
         Object.defineProperty(Object.prototype, 'get', { get() { throw new Error('trap') } });
         Map.prototype.entries = null;
         const p = new Proxy({}, { ownKeys() { throw new Error('nope') }, getPrototypeOf() { throw new Error('nope') } });
         const bomb = { get boom() { throw new Error('getter ran') } };
         console.log([1, 2], { a: 'x' }, new Map([[1, 2]]), bomb);
         console.log(p);
         [p, bomb, 'ok']",
    );
    assert_eq!(o.error, None, "{:?}", o.error);
    assert_eq!(
        texts(&o),
        [
            "[1, 2] { a: \"x\" } Map(1) { 1 => 2 } { boom: [Getter] }",
            "[object]"
        ]
    );
    assert_eq!(
        o.value.as_deref(),
        Some("[[object], { boom: [Getter] }, \"ok\"]")
    );
}

#[test]
fn formatter_errors_from_a_revoked_proxy_fall_back_to_a_tag() {
    let o = run_ok(
        "const { proxy, revoke } = Proxy.revocable({}, {}); revoke(); console.log(proxy); proxy",
    );
    assert_eq!(o.error, None);
    assert_eq!(texts(&o), ["[object]"]);
    assert_eq!(o.value.as_deref(), Some("[object]"));
}

// ── AC8: output cap ──

#[test]
fn output_is_truncated_at_the_line_cap_and_the_value_still_shows() {
    let o = run_with(
        "for (let i = 0; i < 20; i++) console.log('line ' + i); 'done'",
        &RunOptions {
            output_lines: 5,
            ..opts()
        },
    );
    assert!(o.truncated);
    assert_eq!(
        texts(&o),
        ["line 0", "line 1", "line 2", "line 3", "line 4"]
    );
    assert_eq!(o.value.as_deref(), Some("done"), "the last value is exempt");
    assert_eq!(o.error, None);
}

#[test]
fn output_is_truncated_at_the_byte_cap_mid_line_and_errors_still_show() {
    let o = run_with(
        "console.log('abcdef'); console.log('ghijklmnop'); console.log('more'); throw new Error('late')",
        &RunOptions {
            output_bytes: 10,
            ..opts()
        },
    );
    assert!(o.truncated);
    assert_eq!(texts(&o), ["abcdef", "ghij"]);
    assert_eq!(
        err(&o).message,
        "Uncaught Error: late",
        "the error is exempt"
    );
}

#[test]
fn cutting_never_splits_a_surrogate_pair() {
    // 🦡 is two UTF-16 units; 6e5 of them pass the 1 MB cap (in units) mid-pair.
    let o = run_ok("console.log('🦡'.repeat(6e5)); '🦡'.repeat(6e5)");
    assert_eq!(o.error, None, "{:?}", o.error);
    // Cut to the cap in UTF-16 units by the prelude, then to the byte cap by the sink.
    let line = &o.lines[0].text;
    assert!(o.truncated);
    assert!(!line.is_empty());
    assert!(line.trim_end_matches('…').chars().all(|c| c == '🦡'));
    let v = o.value.expect("value");
    assert!(v.ends_with('…'));
    assert!(v.trim_end_matches('…').chars().all(|c| c == '🦡'));
    // Odd caps land mid-pair too.
    for cap in [3, 4, 5] {
        let o = run_with(
            "console.log('🦡🦡🦡🦡'); ['🦡🦡🦡🦡']",
            &RunOptions {
                output_bytes: cap,
                ..opts()
            },
        );
        assert_eq!(o.error, None, "cap {cap}: {:?}", o.error);
        assert!(!o.lines.is_empty() || o.truncated);
        assert!(o.value.is_some(), "cap {cap}");
    }
}

#[test]
fn lone_surrogates_are_shown_as_replacement_characters() {
    let o = run_ok("console.log('a\\uD800b', ['\\uDC00']); '\\uD800'");
    assert_eq!(o.error, None, "{:?}", o.error);
    // Nested strings are JSON-quoted, which escapes a lone surrogate.
    assert_eq!(texts(&o), ["a\u{FFFD}b [\"\\udc00\"]"]);
    assert_eq!(
        o.value.as_deref(),
        Some("\u{FFFD}"),
        "the value doesn't vanish"
    );
    let o = run_ok("throw new Error('bad \\uD83E')");
    assert_eq!(err(&o).message, "Uncaught Error: bad \u{FFFD}");
    let o = run_ok("throw '\\uD83E'");
    assert_eq!(err(&o).message, "Uncaught \u{FFFD}");
}

#[test]
fn huge_nested_strings_are_cut_before_quoting() {
    // Quoting a 40 MB string whole would need another 40 MB: over this limit.
    let o = run_with(
        "const s = 'x'.repeat(4e7); console.log({ s }); [s]",
        &RunOptions {
            memory_limit: 64 * 1024 * 1024,
            output_bytes: 2000,
            ..opts()
        },
    );
    assert_eq!(o.error, None, "{:?}", o.error);
    let v = o.value.expect("value");
    assert!(v.starts_with('[') && v.contains("\"xxx"), "{}", &v[..20]);
    assert!(v.chars().count() <= 2000);
    assert!(o.lines[0].text.contains("s: \"xxx"));
}

#[test]
fn multi_line_values_count_each_line() {
    let o = run_with(
        "console.log('a\\nb\\nc'); console.log('d\\ne')",
        &RunOptions {
            output_lines: 4,
            ..opts()
        },
    );
    assert!(o.truncated);
    assert_eq!(texts(&o), ["a\nb\nc", "d"]);
}

#[test]
fn a_huge_value_is_cut_to_the_cap() {
    let o = run_with(
        "const a = []; for (let i = 0; i < 100000; i++) a.push({ i }); console.log(a); a",
        &RunOptions {
            output_bytes: 2000,
            ..opts()
        },
    );
    assert_eq!(o.error, None);
    let v = o.value.expect("value");
    assert!(v.chars().count() <= 2000, "{}", v.len());
    assert!(v.contains('…'));
}

// ── AC4: timeout and Stop ──

fn short() -> RunOptions {
    RunOptions {
        timeout: Duration::from_millis(250),
        ..RunOptions::default()
    }
}

#[test]
fn busy_loops_time_out_even_inside_try_catch() {
    for code in [
        "while (true) {}",
        "try { while (true) {} } catch {}",
        "try { while (true) {} } finally { while (true) {} }",
        "async function f() { for (;;) await 0 } await f()",
        "(async () => { for (;;) await 0 })(); 1",
        "console.log('before'); for (;;) {}",
    ] {
        let start = Instant::now();
        let o = run_with(code, &short());
        let e = err(&o);
        assert_eq!(e.kind, ErrorKind::Timeout, "{code}: {e:?}");
        assert!(e.message.contains("timed out"), "{}", e.message);
        assert_eq!(o.value, None);
        assert!(start.elapsed() < Duration::from_secs(3), "{code}");
    }
    let o = run_with("console.log('before'); for (;;) {}", &short());
    assert_eq!(texts(&o), ["before"], "output before the timeout is kept");
}

#[test]
fn cancel_flag_stops_a_run() {
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        flag.store(true, Ordering::SeqCst);
    });
    let start = Instant::now();
    let o = run("try { for (;;) {} } catch {}", &opts(), cancel, |_| {
        Ok(None)
    });
    let _ = stopper.join();
    let e = err(&o);
    assert_eq!(e.kind, ErrorKind::Stopped);
    assert_eq!(e.message, "Stopped");
    assert!(
        start.elapsed() < Duration::from_millis(1400),
        "before the timeout"
    );
}

#[test]
fn an_interrupt_before_the_script_starts_is_reported_as_such() {
    // Stop pressed (or the deadline passed) while the prelude is still being set up.
    let o = run("1", &opts(), Arc::new(AtomicBool::new(true)), |_| Ok(None));
    assert_eq!(err(&o).kind, ErrorKind::Stopped, "{:?}", o.error);
    let o = run_with(
        "1",
        &RunOptions {
            timeout: Duration::ZERO,
            ..opts()
        },
    );
    assert_eq!(err(&o).kind, ErrorKind::Timeout, "{:?}", o.error);
}

#[test]
fn a_nul_character_is_a_syntax_error() {
    let o = run_ok("1\0");
    let e = err(&o);
    assert_eq!(e.kind, ErrorKind::Syntax);
    assert_eq!(
        e.message,
        "SyntaxError: the script contains a NUL character"
    );
}

#[test]
fn user_errors_that_look_like_interrupts_stay_user_errors() {
    let o = run_ok("throw new Error('interrupted')");
    assert_eq!(err(&o).kind, ErrorKind::User);
    assert_eq!(err(&o).message, "Uncaught Error: interrupted");
    let o = run_ok("throw new InternalError('interrupted')");
    assert_eq!(err(&o).kind, ErrorKind::User);
}

// ── AC5: memory and stack ──

#[test]
fn memory_blowups_report_out_of_memory_and_the_next_run_works() {
    let small = RunOptions {
        memory_limit: 8 * 1024 * 1024,
        ..opts()
    };
    for code in [
        "'x'.repeat(5e7)",
        "const a = []; for (;;) a.push({ i: a.length })",
        "const m = new Map(); for (let i = 0;; i++) m.set(i, [i])",
        "const a = []; for (;;) a.push('y'.repeat(1000) + a.length)",
    ] {
        let o = run_with(code, &small);
        let e = err(&o);
        assert_eq!(e.kind, ErrorKind::Oom, "{code}: {e:?}");
        assert_eq!(e.message, "Out of memory (the limit is 8 MB)");
    }
    assert_eq!(value("1 + 1"), "2", "the process and engine survive");
}

#[test]
fn null_exception_at_the_memory_limit_is_oom_but_throw_null_is_not() {
    // Seen in the spike: at some limits QuickJS can't even allocate the error and throws null.
    NULL_OOMS.with(|n| n.set(0));
    for limit_kb in [300, 400, 512, 700, 1024] {
        let small = RunOptions {
            memory_limit: limit_kb * 1024,
            ..opts()
        };
        let o = run_with("const a = []; for (;;) a.push({ i: a.length })", &small);
        let e = err(&o);
        assert_eq!(e.kind, ErrorKind::Oom, "{limit_kb} KB: {e:?}");
    }
    assert!(
        NULL_OOMS.with(Cell::get) > 0,
        "at least one limit ends in a null exception, taken for out of memory"
    );
    // classify() directly: a null exception counts as OOM only at the limit.
    let d = |is_null| Described {
        text: "null".to_string(),
        stack: None,
        is_null,
    };
    assert_eq!(
        classify(d(true), true, &opts(), "Uncaught ", false).kind,
        ErrorKind::Oom
    );
    let user = classify(d(true), false, &opts(), "Uncaught ", false);
    assert_eq!(
        (user.kind, user.message.as_str()),
        (ErrorKind::User, "Uncaught null")
    );

    let o = run_ok("throw null");
    assert_eq!(err(&o).kind, ErrorKind::User);
    assert_eq!(err(&o).message, "Uncaught null");
    let o = run_ok("throw 1");
    assert_eq!(err(&o).message, "Uncaught 1");
}

#[test]
fn deep_recursion_is_a_stack_error_and_the_next_run_works() {
    let o = run_ok("function f(n) { return f(n + 1) + 1 }\nf(0)");
    let e = err(&o);
    assert_eq!(e.kind, ErrorKind::Stack, "{e:?}");
    assert_eq!(
        e.message,
        "Uncaught RangeError: Maximum call stack size exceeded"
    );
    // Caught stack overflows are fine.
    assert_eq!(
        value("function g() { g() } try { g() } catch (e) { 'caught' }"),
        "caught"
    );
    assert_eq!(value("2 * 21"), "42");
}

// ── AC6: syntax and runtime errors ──

#[test]
fn syntax_errors_report_message_and_position() {
    let o = run_ok("const a = 1;\nconst b = ;\n");
    let e = err(&o);
    assert_eq!(e.kind, ErrorKind::Syntax, "{e:?}");
    assert!(
        e.message.starts_with("Uncaught SyntaxError: "),
        "{}",
        e.message
    );
    assert_eq!((e.line, e.col), (Some(2), Some(11)));
    assert!(o.lines.is_empty());
}

#[test]
fn runtime_errors_report_message_and_position() {
    let o = run_ok("console.log('start');\nconst x = 1;\n  undefinedFn(x);\n");
    let e = err(&o);
    assert_eq!(e.kind, ErrorKind::User);
    assert_eq!(
        e.message,
        "Uncaught ReferenceError: undefinedFn is not defined"
    );
    assert_eq!((e.line, e.col), (Some(3), Some(3)));
    assert_eq!(texts(&o), ["start"]);

    let o = run_ok("function inner() {\n  throw new TypeError('deep');\n}\ninner();");
    let e = err(&o);
    assert_eq!(e.message, "Uncaught TypeError: deep");
    assert_eq!(e.line, Some(2), "innermost user frame");

    let o = run_ok("await null;\nthrow new Error('after await');");
    let e = err(&o);
    assert_eq!(e.message, "Uncaught Error: after await");
    assert_eq!(e.line, Some(2));

    let o = run_ok("JSON.parse('{x')");
    assert_eq!(
        err(&o).kind,
        ErrorKind::User,
        "runtime SyntaxError is a user error"
    );
    assert!(err(&o).message.starts_with("Uncaught SyntaxError"));
}

#[test]
fn floating_rejections_are_reported_once() {
    let o = run_ok("Promise.reject(new Error('floating')); 'value'");
    assert_eq!(o.value.as_deref(), Some("value"));
    let e = err(&o);
    assert_eq!(e.message, "Uncaught (in promise) Error: floating");
    assert_eq!(e.line, Some(1));
    assert!(o.lines.is_empty());

    // Two: the first is the run's error, the second is logged; none twice.
    let o = run_ok("Promise.reject(1); Promise.reject(2);");
    assert_eq!(err(&o).message, "Uncaught (in promise) 1");
    assert_eq!(texts(&o), ["Uncaught (in promise) 2"]);

    // The top-level rejection is the run's own error, not also a floating one.
    let o = run_ok("await Promise.reject(new Error('top'))");
    assert_eq!(err(&o).message, "Uncaught Error: top");
    assert!(o.lines.is_empty());

    let o = run_ok("throw new Error('sync')");
    assert_eq!(err(&o).message, "Uncaught Error: sync");
    assert!(o.lines.is_empty());
}

// ── AC12: async draining ──

#[test]
fn floating_async_output_is_captured() {
    let o = run_ok("(async () => { await 0; console.log(1) })()");
    assert_eq!(o.error, None);
    assert_eq!(texts(&o), ["1"]);
    let o = run_ok("Promise.resolve().then(() => console.log('then')); console.log('sync')");
    assert_eq!(texts(&o), ["sync", "then"]);
    let o = run_ok("queueMicrotask(() => console.log('micro'))");
    assert_eq!(texts(&o), ["micro"]);
}

#[test]
fn never_settling_await_is_reported() {
    let o = run_ok("await new Promise(() => {})");
    let e = err(&o);
    assert_eq!(e.kind, ErrorKind::User);
    assert_eq!(
        e.message,
        "Script is waiting on a promise that never settles"
    );
}

#[test]
fn rejections_handled_later_are_not_reported() {
    let o = run_ok(
        "const p = Promise.reject(new Error('later'));\n\
         await null;\n\
         p.catch(() => console.log('handled'));\n\
         'ok'",
    );
    assert_eq!(o.error, None, "{:?}", o.error);
    assert_eq!(texts(&o), ["handled"]);
    assert_eq!(o.value.as_deref(), Some("ok"));
}

#[test]
fn a_throwing_microtask_is_reported() {
    let o = run_ok("queueMicrotask(() => { throw new Error('in job') }); 'v'");
    assert_eq!(o.value.as_deref(), Some("v"));
    assert_eq!(err(&o).message, "Uncaught Error: in job");
}

// ── AC7: note() ──

#[test]
fn note_reads_json_parsed_and_markdown_as_text() {
    assert_eq!(value("note('Data').items.reduce((a, b) => a + b, 0)"), "6");
    assert_eq!(
        value("note('DATA').name"),
        "Åsa",
        "title match is case-insensitive"
    );
    assert_eq!(value("typeof note('plan')"), "string");
    assert_eq!(value("note('plan')"), "# Plan\n- [ ] ship");
}

#[test]
fn note_throws_for_unknown_and_invalid_titles() {
    let o = run_ok("const n = note('Nope');");
    let e = err(&o);
    assert_eq!(e.message, "Uncaught Error: Note \"Nope\" not found");
    // QuickJS reports call sites at the callee or one column before it.
    assert_eq!(e.line, Some(1));
    assert!(matches!(e.col, Some(10..=11)), "{:?}", e.col);
    assert_eq!(
        value("try { note('Nope') } catch (e) { e.message }"),
        "Note \"Nope\" not found"
    );
    let o = run_ok("note(42)");
    assert_eq!(
        err(&o).message,
        "Uncaught TypeError: note() expects a note title"
    );
    let o = run_ok("note('broken')");
    assert_eq!(
        err(&o).message,
        "Uncaught SyntaxError: Note \"broken\" is not valid JSON"
    );
    assert_eq!(err(&o).kind, ErrorKind::User);
}

#[test]
fn a_5_mb_json_note_fits_in_memory() {
    let row = r#"{"customer":"Acme","amount":1200.5,"tags":["a","b"]},"#;
    let mut body = String::from("[");
    let mut rows = 0;
    while body.len() < crate::commands::notes::MAX_JSON_BYTES - row.len() - 1 {
        body.push_str(row);
        rows += 1;
    }
    body.pop();
    body.push(']');
    let o = run(
        "const rows = note('big'); rows.reduce((n, r) => n + (r.amount > 0 ? 1 : 0), 0)",
        &RunOptions::default(),
        Arc::new(AtomicBool::new(false)),
        move |_| Ok(Some(NoteValue::Json(body.clone()))),
    );
    assert_eq!(o.error, None);
    assert_eq!(o.value.as_deref(), Some(rows.to_string().as_str()));
}

#[test]
fn note_lookup_errors_are_thrown() {
    let o = run(
        "note('x')",
        &opts(),
        Arc::new(AtomicBool::new(false)),
        |_| Err("Database error: locked".to_string()),
    );
    assert_eq!(err(&o).message, "Uncaught Error: Database error: locked");
}

// ── Helpers ──

#[test]
fn script_position_reads_the_first_user_frame() {
    assert_eq!(
        script_position(
            "Error: x\n    at f (badger_prelude:9:3)\n    at <eval> (eval_script:12:34)\n"
        ),
        Some((12, 34))
    );
    assert_eq!(script_position("    at eval_script:2:11\n"), Some((2, 11)));
    assert_eq!(
        script_position("at eval_script:x\n at eval_script:3:4"),
        Some((3, 4))
    );
    assert_eq!(script_position("no frames"), None);
}

#[test]
fn sink_caps_bytes_on_char_boundaries() {
    let mut sink = Sink::new(&RunOptions {
        output_bytes: 5,
        ..RunOptions::default()
    });
    assert!(!sink.push(Level::Log, "ååå".to_string()));
    assert_eq!(sink.lines[0].text, "åå");
    assert!(sink.truncated);
    assert!(!sink.push(Level::Log, "x".to_string()));
    assert_eq!(sink.lines.len(), 1);
}

/// OOM stress (CVE-2025-69654: an assert in `JS_FreeRuntime` after OOM): many limits and
/// allocation patterns, each in a fresh runtime that is then freed. An abort would kill the
/// test process. Run in release: `cargo test --release js_runner::tests::oom_stress -- --ignored`.
#[test]
#[ignore]
fn oom_stress_never_aborts() {
    let patterns = [
        "const a = []; for (;;) a.push({ i: a.length })",
        "const m = new Map(); for (let i = 0;; i++) m.set(i, [i])",
        "let s = 'x'; for (;;) s += s",
        "'x'.repeat(1e9)",
        "const a = []; for (;;) a.push(new Array(1000).fill(a.length))",
        "const o = {}; for (let i = 0;; i++) o['k' + i] = { i }",
        "const p = []; for (;;) p.push(Promise.resolve(p.length).then((x) => [x]))",
        "async function f(n) { const a = new Array(1e4).fill(n); await 0; return f(n + 1) } await f(0)",
        "const a = []; for (;;) a.push(() => a.length)",
        "function f(a) { return f([a, a]) } f([])",
        "JSON.parse('[' + '1,'.repeat(5e7) + '1]')",
        "const s = new Set(); for (let i = 0;; i++) s.add('v' + i)",
        "const a = []; for (;;) { try { a.push({}) } catch (e) { a.push(e) } }",
        "Promise.reject(new Array(1e7).fill(1)); const a = []; for (;;) a.push({})",
    ];
    let limits_kb = [
        200, 256, 300, 333, 400, 512, 640, 777, 1024, 1500, 2048, 3000, 4096, 8192, 16384, 65536,
    ];
    let mut ooms = 0;
    for limit in limits_kb {
        for code in patterns {
            let o = run_with(
                code,
                &RunOptions {
                    memory_limit: limit * 1024,
                    timeout: Duration::from_secs(5),
                    ..RunOptions::default()
                },
            );
            if o.error.as_ref().is_some_and(|e| e.kind == ErrorKind::Oom) {
                ooms += 1;
            }
        }
    }
    assert!(ooms > 100, "most combinations run out of memory ({ooms})");
    assert_eq!(value("'still alive'"), "still alive");
}
