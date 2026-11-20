//! Tests of what exists only on wasm32: the Worker's own glue (G-6). The configuration it reads from a JS `Env`, its refusals to start,
//! how a queue message's body becomes text, the retry delay, and the JS-backed clock, ids and random bytes. Nothing here runs natively,
//! because every one of them takes a JS value or calls into the JS runtime.
//!
//! They run as the `workers` job of `test.yml`: `scripts/wasm-tests.sh`, which is nextest's `workers` profile through
//! `wasm-bindgen-test-runner` in Node (the runner and its stand-ins for the modules only workerd has: `scripts/wasm-test-runner.sh`).
//! The runner starts once per test function (about two CPU-seconds each), so a function here is a matrix or a loop over a generated domain,
//! never a single case. `Env` is a plain JS object, so a test builds one with the bindings it wants.

use super::*;
// The entrypoints, as `#[event(...)]` leaves them: the macro (worker-macros 0.9) moves each function into a module of its own and exports a wrapper. A
// version that names them otherwise stops this file compiling, which says so.
use crate::_worker_fetch::fetch;
use crate::_worker_queue::queue;
use wasm_bindgen_test::wasm_bindgen_test;
use worker::js_sys::{self, Array, Object, Reflect, Uint8Array};
use worker::wasm_bindgen::{JsCast, JsValue};

// ---------------------------------------------------------------------------------------------
// builders
// ---------------------------------------------------------------------------------------------

/// An `Env` with these bindings, whatever their type: the Worker's own is a JS object of strings (vars and secrets), objects (the
/// bindings) and the like.
fn env_of(bindings: &[(&str, JsValue)]) -> Env {
    let obj = Object::new();
    for (name, value) in bindings {
        Reflect::set(&obj, &JsValue::from_str(name), value).expect("a plain object takes any property");
    }
    obj.unchecked_into()
}

fn strings(vars: &[(&str, &str)]) -> Env {
    env_of(&vars.iter().map(|(name, value)| (*name, JsValue::from_str(value))).collect::<Vec<_>>())
}

fn one(name: &str, value: &str) -> Env {
    strings(&[(name, value)])
}

/// Every string of at most `max_len` characters over `alphabet`, the empty one included.
fn words(alphabet: &[char], max_len: usize) -> Vec<String> {
    let mut all = vec![String::new()];
    let mut layer = vec![String::new()];
    for _ in 0..max_len {
        layer = layer.iter().flat_map(|w| alphabet.iter().map(move |c| format!("{w}{c}"))).collect();
        all.extend(layer.iter().cloned());
    }
    all
}

/// The near misses of a spelling: every deletion of one character, every insertion or substitution of one from `alphabet`, the same in
/// upper case, and with a space on either side. These are the inputs a lenient or a careless reader gets wrong.
fn neighbours(spelling: &str, alphabet: &[char]) -> Vec<String> {
    let chars: Vec<char> = spelling.chars().collect();
    let mut out = vec![spelling.to_uppercase(), format!(" {spelling}"), format!("{spelling} ")];
    for i in 0..chars.len() {
        let mut without = chars.clone();
        without.remove(i);
        out.push(without.into_iter().collect());
        for c in alphabet {
            let mut replaced = chars.clone();
            replaced[i] = *c;
            out.push(replaced.into_iter().collect());
        }
    }
    for i in 0..=chars.len() {
        for c in alphabet {
            let mut inserted = chars.clone();
            inserted.insert(i, *c);
            out.push(inserted.into_iter().collect());
        }
    }
    out
}

/// What this Worker's configuration is with nothing set: the core's defaults, except CSRF enforcement, which the Worker keeps off until the
/// custom frontend replaces the stock one (Q-0050.8).
fn worker_defaults() -> CoreConfig {
    CoreConfig { csrf_enforce: false, ..CoreConfig::default() }
}

// ---------------------------------------------------------------------------------------------
// the variables: text, flags, numbers
// ---------------------------------------------------------------------------------------------

/// The spelling table of a flag, written out here and not read from the code: three that mean on, three that mean off, and every other
/// text is the default.
const FLAG_SPELLINGS: [(&str, bool); 6] =
    [("1", true), ("true", true), ("on", true), ("0", false), ("false", false), ("off", false)];

#[wasm_bindgen_test]
fn flag_reads_exactly_its_six_spellings() {
    let letters: Vec<char> = "01onftrueals TON".chars().collect();
    let mut inputs = words(&letters[..11], 3);
    for (spelling, _) in FLAG_SPELLINGS {
        inputs.push(spelling.to_string());
        inputs.extend(neighbours(spelling, &letters));
    }
    // the generator reaches what matters: every spelling is among the inputs, and they are a minority of them
    let spellings_seen = inputs.iter().filter(|i| FLAG_SPELLINGS.iter().any(|(s, _)| s == i)).count();
    assert!(spellings_seen >= FLAG_SPELLINGS.len(), "{spellings_seen} spellings among the inputs");
    assert!(inputs.len() > 2_000, "only {} inputs", inputs.len());
    for input in &inputs {
        let meaning = FLAG_SPELLINGS.iter().find(|(s, _)| s == input).map(|(_, on)| *on);
        for default in [false, true] {
            assert_eq!(
                flag(&one("X", input), "X", default),
                meaning.unwrap_or(default),
                "{input:?} with the default {default}"
            );
        }
    }
    for default in [false, true] {
        assert_eq!(flag(&strings(&[]), "X", default), default, "unset, default {default}");
    }
}

/// The meaning of a decimal text as an integer in `[lo, hi]`, worked out here digit by digit and not by `str::parse`: an optional sign (a
/// plus always, a minus when the type is signed), then digits and nothing else. `None` for anything else.
fn decimal(text: &str, lo: i128, hi: i128) -> Option<i128> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    // no minus for an unsigned type, not even before a zero ("-0" is not a u32)
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) || (negative && lo >= 0) {
        return None;
    }
    let mut value: i128 = 0;
    for b in digits.bytes() {
        value = value.checked_mul(10)?.checked_add(i128::from(b - b'0'))?;
    }
    let value = if negative { -value } else { value };
    (lo..=hi).contains(&value).then_some(value)
}

#[wasm_bindgen_test]
fn num_reads_decimal_digits_and_nothing_else() {
    let mut inputs = words(&['0', '1', '9', '+', '-', ' ', 'x', '.'], 4);
    // the boundaries of every width: 2^k - 1, 2^k and 2^k + 1, signed both ways, and zeros in front
    for k in 0..=64u32 {
        let power: i128 = 1i128 << k;
        for v in [power - 1, power, power + 1] {
            inputs.extend([v.to_string(), format!("+{v}"), format!("-{v}"), format!("000000000000000000000{v}")]);
        }
    }
    assert!(inputs.iter().any(|i| i == "4294967295") && inputs.iter().any(|i| i == "4294967296"));
    let (mut u32_hits, mut i64_hits) = (0, 0);
    for input in &inputs {
        let env = one("X", input);
        let want_u32 = decimal(input, 0, i128::from(u32::MAX));
        assert_eq!(num::<u32>(&env, "X", 7777), want_u32.map_or(7777, |v| v as u32), "u32 {input:?}");
        let want_i64 = decimal(input, i128::from(i64::MIN), i128::from(i64::MAX));
        assert_eq!(num::<i64>(&env, "X", -7777), want_i64.map_or(-7777, |v| v as i64), "i64 {input:?}");
        u32_hits += usize::from(want_u32.is_some());
        i64_hits += usize::from(want_i64.is_some());
    }
    // the domain has both kinds of input in quantity, so that neither always-the-default nor always-the-number passes
    assert!(u32_hits > 200 && u32_hits < inputs.len() / 2, "{u32_hits} of {} read as u32", inputs.len());
    assert!(i64_hits > u32_hits, "{i64_hits} read as i64");
    assert_eq!(num::<u32>(&strings(&[]), "X", 5), 5, "unset");
}

#[wasm_bindgen_test]
fn a_binding_that_is_not_a_string_is_unset() {
    // an object is what a real binding is (a queue, a bucket): a var of the same name must not read as its text
    let not_strings: [JsValue; 5] =
        [JsValue::from_f64(1.0), JsValue::TRUE, JsValue::UNDEFINED, Object::new().into(), Array::new().into()];
    for value in not_strings {
        let env = env_of(&[("X", value.clone())]);
        assert_eq!(text(&env, "X"), None, "{value:?}");
        assert!(flag(&env, "X", true) && !flag(&env, "X", false), "{value:?}");
        assert_eq!(num::<u32>(&env, "X", 5), 5, "{value:?}");
    }
    assert_eq!(text(&one("X", "a"), "X").as_deref(), Some("a"));
    assert_eq!(
        text(&one("X", ""), "X").as_deref(),
        Some(""),
        "an empty string is a value, and each reader decides what it means"
    );
}

// ---------------------------------------------------------------------------------------------
// core_config: each variable is its own field
// ---------------------------------------------------------------------------------------------

type Change = fn(&mut CoreConfig);

/// The variables `core_config` reads, with a value and what that value sets: the table of the deploy documentation, written out here. A
/// misspelt name in the Worker does not fail anywhere else: the field keeps its default and the deployment carries on without the setting.
fn core_vars() -> Vec<(&'static str, &'static str, Change)> {
    vec![
        ("THRUST_OUTBOX_FASTPATH", "off", |c| c.outbox_fastpath = false),
        ("THRUST_SIGNIN_RATE_LIMIT", "7", |c| c.sign_in_rate_limit = 7),
        ("THRUST_CSRF_ENFORCE", "on", |c| c.csrf_enforce = true),
        ("THRUST_PUBLIC_BASE_URL", "https://thrust.example///", |c| {
            c.public_base_url = Some("https://thrust.example".into())
        }),
        ("THRUST_PASSWORD_RESET_TTL_SECS", "1234", |c| c.password_reset_ttl_secs = 1234),
        ("THRUST_PASSWORD_RESET_COOLDOWN_SECS", "99", |c| c.password_reset_cooldown_secs = 99),
        ("THRUST_ARGON2_M_KIB", "8", |c| c.argon2.m_cost_kib = 8),
        ("THRUST_ARGON2_T", "3", |c| c.argon2.t_cost = 3),
        ("THRUST_ARGON2_P", "2", |c| c.argon2.p_cost = 2),
    ]
}

/// `CoreConfig` has no `PartialEq`; its `Debug` names every field.
fn shown(cfg: &CoreConfig) -> String {
    format!("{cfg:?}")
}

#[wasm_bindgen_test]
fn core_config_maps_each_variable_to_its_own_field() {
    assert_eq!(shown(&core_config(&strings(&[]))), shown(&worker_defaults()), "nothing set");
    let vars = core_vars();
    assert_eq!(vars.len(), 9);
    for (name, value, change) in &vars {
        let mut want = worker_defaults();
        change(&mut want);
        assert_ne!(
            shown(&want),
            shown(&worker_defaults()),
            "{name}={value:?} must change something, or this case proves nothing"
        );
        assert_eq!(shown(&core_config(&one(name, value))), shown(&want), "{name}={value:?}");
    }
    // all of them at once: two fields reading one variable, or one reading two, show here
    let mut all = worker_defaults();
    for (_, _, change) in &vars {
        change(&mut all);
    }
    let every: Vec<(&str, &str)> = vars.iter().map(|(name, value, _)| (*name, *value)).collect();
    assert_eq!(shown(&core_config(&strings(&every))), shown(&all), "all nine");
    // a value that does not read is the default, for every variable that is a flag or a number (the base URL is any text)
    for (name, _, _) in vars.iter().filter(|(name, _, _)| *name != "THRUST_PUBLIC_BASE_URL") {
        assert_eq!(
            shown(&core_config(&one(name, "not-a-value"))),
            shown(&worker_defaults()),
            "{name} that does not read"
        );
    }
    // an empty base URL is no base URL, so that no link can be made
    assert_eq!(shown(&core_config(&one("THRUST_PUBLIC_BASE_URL", ""))), shown(&worker_defaults()));
}

// ---------------------------------------------------------------------------------------------
// refusals to start
// ---------------------------------------------------------------------------------------------

#[wasm_bindgen_test]
fn oidc_is_all_three_settings_or_none() {
    let names = ["THRUST_OIDC_ISSUER", "THRUST_OIDC_CLIENT_ID", "THRUST_OIDC_REDIRECT_URI"];
    let mut counts = [0usize; 4];
    // each of the three unset, empty or a value, with and without the client secret: 54 environments
    for picks in (0..27).map(|n| [n % 3, (n / 3) % 3, (n / 9) % 3]) {
        for secret in [false, true] {
            let mut bindings: Vec<(&str, JsValue)> = Vec::new();
            for (name, pick) in names.iter().zip(picks) {
                match pick {
                    1 => bindings.push((name, JsValue::from_str(""))),
                    2 => bindings.push((name, JsValue::from_str("x"))),
                    _ => {}
                }
            }
            if secret {
                bindings.push(("THRUST_OIDC_CLIENT_SECRET", JsValue::from_str("s")));
            }
            // an empty setting is an unset one
            let set = picks.iter().filter(|p| **p == 2).count();
            counts[set] += 1;
            match oidc_provider(&env_of(&bindings)) {
                Ok(Some(_)) => assert_eq!(set, 3, "{picks:?} secret {secret}: a provider with {set} of 3"),
                Ok(None) => assert_eq!(set, 0, "{picks:?} secret {secret}: no provider with {set} of 3"),
                Err(e) => {
                    assert!(set == 1 || set == 2, "{picks:?} secret {secret}: refused with {set} of 3");
                    assert!(e.to_string().contains("must be set together"), "{e}");
                }
            }
        }
    }
    assert!(counts.iter().all(|n| *n > 0), "the environments reach 0, 1, 2 and 3 settings: {counts:?}");
}

#[wasm_bindgen_test]
fn rate_windows_are_ten_or_sixty_seconds() {
    // the sign-in window, every second up to 200 and the largest one: a Workers Rate Limiting binding counts over 10 s or 60 s (ADR-0033)
    for secs in (0..=200u64).chain([u64::MAX]) {
        let cfg = CoreConfig { sign_in_rate_window_secs: secs, ..worker_defaults() };
        let result = check_rate_windows(&strings(&[]), &cfg);
        assert_eq!(result.is_ok(), secs == 10 || secs == 60, "sign-in window of {secs} s");
        if let Err(e) = result {
            assert!(e.to_string().contains("the sign-in rate window"), "{e}");
        }
    }
    // THRUST_V1_RATE_LIMIT, in Plane's spelling: what it is read as (`None`: it does not read, and the allowance is Plane's own, 60 a minute), and
    // whether the window it names is one the binding can count over: only a period that reads and is not a minute is refused.
    // A hand-written table, Rule 9 (b): examples of the spelling that `V1Config::parse` states (DRF's `N/period`, the period read from its first
    // letter: crates/api/v1/src/ctx.rs). The parser is tested in thrust-api-v1; this pins what the Worker does with what it reads.
    // (the spelling, what it reads as, whether the binding can count over the window it names)
    type Spec = (&'static str, Option<(u32, u64)>, bool);
    let specs: [Spec; 14] = [
        ("100/minute", Some((100, 60)), true),
        ("100/m", Some((100, 60)), true),
        ("100", Some((100, 60)), true),
        (" 100 / minute ", Some((100, 60)), true),
        ("100/second", Some((100, 1)), false),
        ("100/s", Some((100, 1)), false),
        ("100/hour", Some((100, 3600)), false),
        ("7/h", Some((7, 3600)), false),
        ("100/day", Some((100, 86_400)), false),
        ("100/week", None, true),
        ("100/Minute", None, true),
        ("lots/minute", None, true),
        ("-5/minute", None, true),
        ("", None, true),
    ];
    for (spec, read_as, accepted) in specs {
        let env = one("THRUST_V1_RATE_LIMIT", spec);
        let result = check_rate_windows(&env, &worker_defaults());
        assert_eq!(result.is_ok(), accepted, "THRUST_V1_RATE_LIMIT={spec:?}");
        if let Err(e) = result {
            assert!(e.to_string().contains("THRUST_V1_RATE_LIMIT"), "{e}");
        }
        let cfg = v1_config(&env);
        assert_eq!(
            (cfg.rate_limit, cfg.rate_window_secs),
            read_as.unwrap_or((60, 60)),
            "THRUST_V1_RATE_LIMIT={spec:?} as the routes read it"
        );
    }
    assert!(check_rate_windows(&strings(&[]), &worker_defaults()).is_ok(), "nothing set");
    let unset = v1_config(&strings(&[]));
    assert_eq!((unset.rate_limit, unset.rate_window_secs), (60, 60), "Plane's own allowance when nothing is set");
}

fn background() -> Arc<dyn Background> {
    Arc::new(LocalBackground)
}

#[wasm_bindgen_test]
fn the_composition_root_refuses_what_it_cannot_serve() {
    let refusal = |bindings: &[(&str, &str)]| match build_app(&strings(bindings), background()) {
        Ok(_) => String::new(),
        Err(e) => e.to_string(),
    };
    assert!(refusal(&[("THRUST_ADAPTERS", "nope")]).contains("'nope' is not compiled into this bundle"));
    // the set that is compiled in by default builds on wasm32, in the isolate, and counts no database statements
    #[cfg(feature = "memory")]
    match build_app(&one("THRUST_ADAPTERS", "memory"), background()) {
        Ok((_, round_trips)) => assert!(round_trips.is_none()),
        Err(e) => panic!("the memory set did not build: {e}"),
    }
    // the cloudflare set checks its windows before it asks for a binding: a window it cannot count over is why it does not start
    let bad = refusal(&[("THRUST_ADAPTERS", "cloudflare"), ("THRUST_V1_RATE_LIMIT", "10/second")]);
    assert!(bad.contains("THRUST_V1_RATE_LIMIT") && bad.contains("10 s or 60 s"), "{bad:?}");
    // a window it can, and the first thing the environment lacks is the database binding
    let missing = refusal(&[("THRUST_ADAPTERS", "cloudflare"), ("THRUST_V1_RATE_LIMIT", "100/minute")]);
    assert!(missing.contains("HYPERDRIVE") && !missing.contains("THRUST_V1_RATE_LIMIT"), "{missing:?}");
    // a half-configured login is not a Worker with no login
    let half = refusal(&[("THRUST_ADAPTERS", "cloudflare"), ("THRUST_OIDC_ISSUER", "https://idp.example")]);
    assert!(half.contains("must be set together"), "{half:?}");
}

/// The rest of what the Worker reads from its environment: which adapter set, the HTTP layer's settings, the mail service, the version that is running.
#[wasm_bindgen_test]
fn the_rest_of_the_environment_is_read_by_name() {
    // the adapter set: the one compiled in by default unless a variable names another
    let default_set = if cfg!(feature = "memory") { "memory" } else { "cloudflare" };
    assert_eq!(adapter_set(&strings(&[])), default_set);
    assert_eq!(adapter_set(&one("THRUST_ADAPTERS", "cloudflare")), "cloudflare");
    assert_eq!(
        adapter_set(&one("THRUST_ADAPTERS", "anything")),
        "anything",
        "the name is checked when the app is built, not here"
    );

    // the HTTP layer: each variable is its own field, and `target` is always the Worker
    let api = |bindings: &[(&str, &str)]| format!("{:?}", api_config(&strings(bindings), "A"));
    let expected = |test_routes, admin: Option<&str>, operator_routes, operator: Option<&str>| {
        format!(
            "{:?}",
            ApiConfig {
                test_routes,
                admin_token: admin.map(String::from),
                operator_routes,
                operator_token: operator.map(String::from),
                target: "worker".into(),
                adapters: "A".into(),
            }
        )
    };
    assert_eq!(
        api(&[]),
        expected(false, None, false, None),
        "nothing set: no test routes, no operator routes, no tokens"
    );
    assert_eq!(api(&[("THRUST_TEST_ROUTES", "on")]), expected(true, None, false, None));
    assert_eq!(api(&[("THRUST_ADMIN_TOKEN", "admin")]), expected(false, Some("admin"), false, None));
    assert_eq!(api(&[("THRUST_OPERATOR_ROUTES", "1")]), expected(false, None, true, None));
    assert_eq!(api(&[("THRUST_OPERATOR_TOKEN", "operator")]), expected(false, None, false, Some("operator")));
    assert_eq!(
        api(&[
            ("THRUST_TEST_ROUTES", "true"),
            ("THRUST_ADMIN_TOKEN", "a"),
            ("THRUST_OPERATOR_ROUTES", "on"),
            ("THRUST_OPERATOR_TOKEN", "o")
        ]),
        expected(true, Some("a"), true, Some("o")),
        "all four"
    );

    // the mail service: none without a sender; a sender needs its binding; a name needs a sender; a sender has to be one plain address
    assert!(matches!(email_sender(&strings(&[])), Ok(None)), "no sender, no mail service");
    let refusal = |bindings: &[(&str, &str)]| match email_sender(&strings(bindings)) {
        Ok(_) => String::new(),
        Err(e) => e.to_string(),
    };
    assert!(
        refusal(&[("THRUST_EMAIL_FROM", "no-reply@notify.example")]).contains("`EMAIL` send_email binding"),
        "a sender and no binding"
    );
    assert!(refusal(&[("THRUST_EMAIL_FROM_NAME", "Thrust")]).contains("THRUST_EMAIL_FROM"), "a name and no sender");
    assert!(
        refusal(&[("THRUST_EMAIL_FROM", "not an address")]).contains("THRUST_EMAIL_FROM"),
        "a sender that is not an address"
    );
    // with the binding workerd gives it (a `SendEmail`), the service is built
    let binding = js_sys::Function::new_no_args("class SendEmail { send() {} } return new SendEmail();")
        .call0(&JsValue::NULL)
        .expect("a fake binding");
    let env = env_of(&[("THRUST_EMAIL_FROM", JsValue::from_str("no-reply@notify.example")), ("EMAIL", binding)]);
    assert!(matches!(email_sender(&env), Ok(Some(_))), "a sender and its binding");

    // the version that is running: empty when the binding is not declared, the binding's own tag and id when it is
    assert_eq!(version(&strings(&[])), (String::new(), String::new()));
    let metadata = Object::new();
    Reflect::set(&metadata, &JsValue::from_str("tag"), &JsValue::from_str("v1.2.3")).expect("set");
    Reflect::set(&metadata, &JsValue::from_str("id"), &JsValue::from_str("0b1c")).expect("set");
    assert_eq!(
        version(&env_of(&[("CF_VERSION_METADATA", metadata.into())])),
        ("v1.2.3".to_string(), "0b1c".to_string())
    );
}

#[wasm_bindgen_test]
async fn a_dead_letter_needs_the_queue_name_and_the_queue() {
    let letter = NewDeadLetter {
        source_id: Some("message-1".into()),
        topic: "events".into(),
        event_id: None,
        body: "not an envelope".into(),
        error: "it would not decode".into(),
        attempts: Some(4),
    };
    // without the queue's name the consumer could not tell the dead-letter queue's batches from the events queue's
    for env in [strings(&[]), one("THRUST_EVENTS_DLQ", "")] {
        let e = send_dead_letter(&env, &letter).await.expect_err("no queue name");
        assert!(e.to_string().contains("THRUST_EVENTS_DLQ is not set"), "{e}");
    }
    // the name without the binding is a different refusal
    let e = send_dead_letter(&one("THRUST_EVENTS_DLQ", "dlq"), &letter).await.expect_err("no binding");
    assert!(e.to_string().contains("EVENTS_DLQ") && !e.to_string().contains("is not set"), "{e}");
    // with both, one message is sent, as text, and it is the dead letter that was meant
    let queue = fake_queue();
    let env = env_of(&[("THRUST_EVENTS_DLQ", JsValue::from_str("dlq")), ("EVENTS_DLQ", queue.clone())]);
    send_dead_letter(&env, &letter).await.expect("sent");
    let sent = sent_to(&queue);
    assert_eq!(sent.len(), 1);
    for part in ["message-1", "not an envelope", "it would not decode"] {
        assert!(sent[0].contains(part), "{part:?} is not in {:?}", sent[0]);
    }
}

// ---------------------------------------------------------------------------------------------
// the entrypoints, in the isolate: `fetch` with real requests, `queue` with a fake batch (the memory adapter set, which is this crate's default)
// ---------------------------------------------------------------------------------------------

/// A queue binding as workerd gives one, recording what was sent. The class is named as workerd names it (`WorkerQueue`): `Env` tells a binding by its
/// constructor's name, and refuses a `Queue` ("cannot be cast to the type WorkerQueue").
fn fake_queue() -> JsValue {
    js_sys::Function::new_no_args(
        "class WorkerQueue { constructor() { this.sent = []; } send(body, options) { this.sent.push(body); return Promise.resolve(); } } return new WorkerQueue();",
    )
    .call0(&JsValue::NULL)
    .expect("a fake queue")
}

/// The messages a fake queue was sent, as the text each was.
fn sent_to(queue: &JsValue) -> Vec<String> {
    let sent =
        Array::from(&Reflect::get(queue, &JsValue::from_str("sent")).expect("a fake queue records what it was sent"));
    sent.iter().map(|m| m.as_string().expect("the Worker sends text")).collect()
}

/// A message as a consumer is handed one, counting what was done with it.
fn fake_message(id: &str, body: &str, attempts: u32) -> JsValue {
    js_sys::Function::new_with_args(
        "id, body, attempts",
        "return { id, body, attempts, timestamp: new Date(), acked: 0, retried: 0, retryOptions: null, ack() { this.acked += 1; }, retry(options) { this.retried += 1; this.retryOptions = options; } };",
    )
    .call3(&JsValue::NULL, &JsValue::from_str(id), &JsValue::from_str(body), &JsValue::from_f64(f64::from(attempts)))
    .expect("a fake message")
}

fn field(object: &JsValue, name: &str) -> JsValue {
    Reflect::get(object, &JsValue::from_str(name)).expect("a field")
}

/// `(acknowledged, delivered again)`: how many times each was asked of a fake message.
fn handled(message: &JsValue) -> (u32, u32) {
    (
        field(message, "acked").as_f64().expect("a count") as u32,
        field(message, "retried").as_f64().expect("a count") as u32,
    )
}

/// A batch as workerd hands it to the `queue` export, which is what the exports of `#[event(queue)]` and `#[event(fetch)]` take: the platform's own types.
fn batch_of(queue: &str, messages: &[&JsValue]) -> worker::worker_sys::MessageBatch {
    let batch = Object::new();
    Reflect::set(&batch, &JsValue::from_str("queue"), &JsValue::from_str(queue)).expect("set");
    Reflect::set(&batch, &JsValue::from_str("messages"), &messages.iter().map(|m| (*m).clone()).collect::<Array>())
        .expect("set");
    batch.unchecked_into()
}

fn context() -> worker::worker_sys::Context {
    let ctx = js_sys::Function::new_no_args("return { waitUntil() {}, passThroughOnException() {} };")
        .call0(&JsValue::NULL)
        .expect("a fake context");
    ctx.unchecked_into()
}

#[wasm_bindgen_test]
async fn the_queue_consumer_acknowledges_delivers_again_and_sets_aside() {
    let dlq = fake_queue();
    let events_env = || env_of(&[("THRUST_EVENTS_DLQ", JsValue::from_str("events-dlq")), ("EVENTS_DLQ", dlq.clone())]);

    // a message of the events queue that is not an event is set aside: sent to the dead-letter queue as text, and acknowledged
    let garbage = fake_message("m1", "not an event", 1);
    queue(batch_of("events", &[&garbage]), events_env(), context()).await.expect("the batch was handled");
    assert_eq!(handled(&garbage), (1, 0), "set aside, and not delivered again");
    let sent = sent_to(&dlq);
    assert_eq!(sent.len(), 1, "one dead letter");
    assert!(sent[0].contains("m1") && sent[0].contains("not an event"), "{:?}", sent[0]);

    // the same text on the dead-letter queue's own batch is written down as a row (the memory database here), not sent on again
    let from_dlq = fake_message("m2", "not an event", 1);
    queue(batch_of("events-dlq", &[&from_dlq]), events_env(), context()).await.expect("the batch was handled");
    assert_eq!(handled(&from_dlq), (1, 0), "written, and acknowledged");
    assert_eq!(sent_to(&dlq).len(), 1, "a dead letter's own queue sets nothing aside");

    // what the Worker itself set aside comes back on that queue as a dead letter, and is written the same way
    let letter = DeadLetterMessage::new(
        "m1".into(),
        "events".into(),
        None,
        "not an event".into(),
        "it did not decode".into(),
        1,
    )
    .to_text()
    .expect("text");
    let returned = fake_message("m3", &letter, 1);
    queue(batch_of("events-dlq", &[&returned]), events_env(), context()).await.expect("the batch was handled");
    assert_eq!(handled(&returned), (1, 0));

    // a Worker that does not know the dead-letter queue's name sets nothing aside: the message is delivered again, later, and not lost or acknowledged
    for attempts in [1, 2, 3] {
        let stuck = fake_message("m4", "not an event", attempts);
        queue(batch_of("events", &[&stuck]), env_of(&[("EVENTS_DLQ", dlq.clone())]), context())
            .await
            .expect("the batch was handled");
        assert_eq!(handled(&stuck), (0, 1), "attempt {attempts}: delivered again, not acknowledged");
        let delay = field(&field(&stuck, "retryOptions"), "delaySeconds").as_f64().expect("a delay");
        assert_eq!(
            delay as u64,
            thrust_ports::retry_delay(attempts).as_secs(),
            "attempt {attempts}: the delay is the policy's"
        );
    }
    assert_eq!(sent_to(&dlq).len(), 1, "and sends nothing");

    // every message of a batch is handled, each on its own
    let (a, b) = (fake_message("m5", "first", 1), fake_message("m6", "second", 1));
    queue(batch_of("events", &[&a, &b]), events_env(), context()).await.expect("the batch was handled");
    assert_eq!((handled(&a), handled(&b)), ((1, 0), (1, 0)));
    assert_eq!(sent_to(&dlq).len(), 3, "two more dead letters");
}

/// A request through `fetch`, which is what the platform calls: a real `Request`, the memory set, and no database.
async fn served(method: &str, url: &str, body: Option<&Uint8Array>) -> web_sys::Response {
    let init = web_sys::RequestInit::new();
    init.set_method(method);
    if let Some(body) = body {
        init.set_body(body);
    }
    let request = web_sys::Request::new_with_str_and_init(url, &init).expect("a request");
    // the export answers with a JS value, which for a Worker is a `Response`
    fetch(request, strings(&[("THRUST_ADAPTERS", "memory")]), context()).await.expect("a response").unchecked_into()
}

#[wasm_bindgen_test]
async fn the_entrypoint_routes_and_limits_a_body_to_eight_mebibytes() {
    // the three routers merged into one answer: a liveness probe reaches its handler and gets its own answer
    let health = served("GET", "http://worker.test/healthz", None).await;
    assert_eq!(health.status(), 200);
    let text = worker::wasm_bindgen_futures::JsFuture::from(health.text().expect("a body")).await.expect("text");
    assert_eq!(text.as_string().as_deref(), Some("ok"));
    assert_eq!(served("GET", "http://worker.test/no-such-route", None).await.status(), 404);

    // a body of exactly the ceiling gets past the Worker's limit (whatever the route then says of it), and one byte more is refused as too large;
    // the ceiling is the router's own (`DefaultBodyLimit`, 8 MiB), which the constant's comment says
    let ceiling = 8 * 1024 * 1024u32;
    for (len, refused) in
        [(0, false), (1, false), (ceiling - 1, false), (ceiling, false), (ceiling + 1, true), (ceiling + 4096, true)]
    {
        let body = Uint8Array::new_with_length(len);
        let answer = served("POST", "http://worker.test/no-such-route", Some(&body)).await;
        assert_eq!(answer.status() == 413, refused, "a body of {len} bytes answered {}", answer.status());
    }
}

// ---------------------------------------------------------------------------------------------
// the queue consumer's two helpers
// ---------------------------------------------------------------------------------------------

#[wasm_bindgen_test]
fn the_retry_delay_is_whole_seconds_up_to_the_largest_u32() {
    let ceiling = u64::from(u32::MAX);
    for secs in [0, 1, 2, 5, 59, 60, 3600, ceiling - 1, ceiling, ceiling + 1, u64::MAX / 2, u64::MAX] {
        for nanos in [0, 1, 500_000_000, 999_999_999] {
            let options =
                serde_json::to_value(retry_in(std::time::Duration::new(secs, nanos))).expect("options serialize");
            assert_eq!(
                options["delaySeconds"],
                serde_json::json!(secs.min(ceiling)),
                "{secs} s {nanos} ns, as the queue reads it"
            );
        }
    }
}

fn message_with_body(body: &JsValue) -> RawMessage {
    let message = Object::new();
    Reflect::set(&message, &JsValue::from_str("body"), body).expect("a plain object takes any property");
    RawMessage::from(message.unchecked_into::<worker::worker_sys::Message>())
}

#[wasm_bindgen_test]
fn a_message_body_becomes_text() {
    let object = Object::new();
    Reflect::set(&object, &JsValue::from_str("a"), &JsValue::from_f64(1.0)).expect("set");
    let nested = Array::of2(&JsValue::from_str("x"), &Array::of1(&JsValue::NULL));
    let cases: Vec<(JsValue, &str)> = vec![
        (JsValue::from_str("hello"), "hello"),
        (JsValue::from_str(""), ""),
        (JsValue::from_str("{\"already\":\"json\"}"), "{\"already\":\"json\"}"),
        (object.into(), "{\"a\":1}"),
        (nested.into(), "[\"x\",[null]]"),
        (JsValue::from_f64(42.5), "42.5"),
        (JsValue::TRUE, "true"),
        (JsValue::NULL, "null"),
    ];
    for (body, text) in cases {
        assert_eq!(body_text(&message_with_body(&body)), text, "{body:?}");
    }
    // a body JSON cannot write (a bigint) is shown as the runtime shows it, not lost and not a crash
    let big = JsValue::from(js_sys::BigInt::from(5i64));
    assert!(body_text(&message_with_body(&big)).starts_with("JsValue("));
}

// ---------------------------------------------------------------------------------------------
// the JS runtime behind the ports
// ---------------------------------------------------------------------------------------------

#[wasm_bindgen_test]
fn the_clock_ids_and_random_bytes_come_from_the_js_runtime() {
    let before = worker::Date::now().as_millis() as i64;
    let now = JsClock.now().timestamp_millis();
    let mut ids = std::collections::HashSet::new();
    for _ in 0..2_000 {
        let id = JsIdGen.new_uuid();
        assert_eq!(id.get_version_num(), 7, "{id}");
        assert!(ids.insert(id), "{id} was made twice");
        let (secs, nanos) = id.get_timestamp().expect("a v7 id carries a time").to_unix();
        let ms = (secs * 1000 + u64::from(nanos) / 1_000_000) as i64;
        assert!(before <= ms, "{id} is dated {ms}, before the loop began at {before}");
    }
    let after = worker::Date::now().as_millis() as i64;
    assert!(before <= now && now <= after, "{now} is not in [{before}, {after}]");
    for id in &ids {
        let (secs, nanos) = id.get_timestamp().expect("a v7 id carries a time").to_unix();
        assert!(
            (secs * 1000 + u64::from(nanos) / 1_000_000) as i64 <= after,
            "{id} is dated after the loop ended at {after}"
        );
    }
    let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
    JsRng.fill(&mut a).expect("random bytes");
    JsRng.fill(&mut b).expect("random bytes");
    assert_ne!(a, [0u8; 32]);
    assert_ne!(a, b);
    JsRng.fill(&mut []).expect("an empty buffer is filled");
}
