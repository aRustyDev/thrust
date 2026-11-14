//! The SMTP adapter's conversation with a server, against a scripted one in the test's own process (`sink.rs`): hermetic, so it runs in the
//! default loop. What it settles is what the adapter decides and a real relay will not do on demand: how each reply code is read, that what
//! arrives is what was given, that nothing that is not one address becomes a recipient or a header, and what happens when the relay is silent,
//! gone or refusing. `smtp_sink` is the same adapter against Mailpit, a server nobody here wrote, with the TLS and the login this one cannot show.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod sink;

use std::sync::OnceLock;
use std::time::Duration;

use mailparse::MailHeaderMap;
use proptest::prelude::*;
use sink::{Behaviour, Reply, Script, Sink};
use thrust_adapters_selfhosted::smtp::{SmtpConfig, SmtpEmail, SmtpTls};
use thrust_ports::{EmailError, EmailMessage, EmailSender};

const FROM: &str = "Thrust <no-reply@example.com>";

fn config(sink: &Sink) -> SmtpConfig {
    SmtpConfig {
        host: sink.host(),
        port: sink.port(),
        tls: SmtpTls::None,
        credentials: None,
        from: FROM.parse().unwrap(),
        ca_file: None,
        timeout: Duration::from_secs(10),
    }
}

fn message(to: &str) -> EmailMessage {
    EmailMessage { to: to.into(), subject: "Reset your password".into(), text: "Follow the link.\n".into(), html: None }
}

/// What the test sees of a message: its headers (name, decoded value) and the parsed whole.
fn parse(raw: &[u8]) -> mailparse::ParsedMail<'_> {
    mailparse::parse_mail(raw).expect("what the adapter sent parses as a message")
}

/// Whitespace of every kind is one space, and the ends are trimmed: how RFC 5322 lets a header be folded and a body re-wrapped, so a
/// comparison across the wire does not fail on a fold. A dropped line, a moved dot or an injected header still does.
fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// No whitespace at all. A subject that has to be sent as encoded words (a line break or a non-ASCII character in it) comes back without the
/// white space between two of them, which RFC 2047 6.2 tells a reader to ignore: `lettre` splits at the spaces and a reader drops them,
/// so a run of spaces in an awkward subject is gone on the other side. Seen by this very test (`"\r.  a\n"` came back `".a"`); cosmetic.
fn bare(s: &str) -> String {
    s.split_whitespace().collect()
}

#[derive(Clone, Copy, Debug)]
enum Step {
    Greeting,
    Mail,
    Rcpt,
    Data,
    End,
}

fn refusing(step: Step, code: u16) -> Script {
    let mut script = Script::default();
    let reply = Reply::new(code, "scripted");
    match step {
        Step::Greeting => script.greeting = reply,
        Step::Mail => script.mail = reply,
        Step::Rcpt => script.rcpt = reply,
        Step::Data => script.data = reply,
        Step::End => script.end = reply,
    }
    script
}

/// The control of everything below: with nothing refused, the same message goes through, and the sink holds it. Without it "every refusal is
/// refused" could be an adapter that never sends.
#[tokio::test]
async fn a_server_that_takes_the_message_is_answered_with_the_messages_id() {
    let sink = Sink::start(Behaviour::Scripted(Script::default())).await;
    let email = SmtpEmail::new(&config(&sink)).unwrap();
    let id = email.send(&message("alice@example.org")).await.expect("accepted");
    assert!(id.starts_with('<') && id.ends_with("@example.com>"), "an id in the sender's domain: {id}");
    assert_eq!(sink.messages().len(), 1);
}

/// RFC 5321's reply classes, at every step of the conversation where a server may refuse, for every code in them: a 4xx is "try again later" and
/// a 5xx is "no", whatever the number after it. The codes are the ones the grammar has (RFC 5321 4.2.1: the second digit is 0 to 5), so 4 x 6 x 10
/// of each class; exhaustive over 5 steps by 120 codes, a finite domain, so no sampling.
#[tokio::test]
async fn every_reply_code_at_every_step_is_an_outage_or_a_refusal_by_its_first_digit() {
    let mut wrong: Vec<String> = Vec::new();
    let mut cases = 0;
    for step in [Step::Greeting, Step::Mail, Step::Rcpt, Step::Data, Step::End] {
        for code in (400..=459u16).chain(500..=559) {
            cases += 1;
            let sink = Sink::start(Behaviour::Scripted(refusing(step, code))).await;
            let email = SmtpEmail::new(&config(&sink)).unwrap();
            let got = email.send(&message("alice@example.org")).await;
            let fine =
                matches!((code / 100, &got), (4, Err(EmailError::Unavailable(_))) | (5, Err(EmailError::Rejected(_))));
            if !fine {
                wrong.push(format!("{step:?} {code}: {got:?}"));
            }
        }
    }
    assert_eq!(cases, 600, "the domain is 5 steps by 120 codes");
    assert!(
        wrong.is_empty(),
        "{} of {cases} cases were read the wrong way, the first few: {:#?}",
        wrong.len(),
        &wrong[..wrong.len().min(8)]
    );
}

/// A reply that is not in the grammar (a second digit above 5) is neither a refusal nor an acceptance: `lettre` cannot read it and says so, and
/// it is an outage, because nothing in it says the message would be refused again. Observed, not chosen: pinned so a change of reading shows.
#[tokio::test]
async fn a_reply_that_is_not_a_code_is_an_outage() {
    for code in [560u16, 599, 460, 499, 600, 700] {
        let sink = Sink::start(Behaviour::Scripted(refusing(Step::Rcpt, code))).await;
        let got = SmtpEmail::new(&config(&sink)).unwrap().send(&message("alice@example.org")).await;
        assert!(matches!(got, Err(EmailError::Unavailable(_))), "{code}: {got:?}");
    }
}

/// The sink answers nothing, or hangs up, or is not there: the relay is out, and that may pass. And the deadline holds: a relay that is up and
/// silent does not hold the request open for longer than the configured time (50 ms here, which is a wait for nothing to happen and so cannot
/// be made shorter by a clock).
#[tokio::test]
async fn a_relay_that_is_silent_gone_or_not_there_is_an_outage() {
    let silent = Sink::start(Behaviour::Silent).await;
    let mut cfg = config(&silent);
    cfg.timeout = Duration::from_millis(50);
    let started = std::time::Instant::now();
    let got = SmtpEmail::new(&cfg).unwrap().send(&message("alice@example.org")).await;
    assert!(matches!(got, Err(EmailError::Unavailable(_))), "silence is an outage: {got:?}");
    assert!(started.elapsed() < Duration::from_secs(5), "the deadline did not hold: {:?}", started.elapsed());

    let hung_up = Sink::start(Behaviour::HangUp).await;
    let got = SmtpEmail::new(&config(&hung_up)).unwrap().send(&message("alice@example.org")).await;
    assert!(matches!(got, Err(EmailError::Unavailable(_))), "a hang-up is an outage: {got:?}");

    // a port nobody listens on: bind one, learn its number, let it go
    let gone = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut cfg = config(&silent);
        cfg.port = port;
        cfg
    };
    let got = SmtpEmail::new(&gone).unwrap().send(&message("alice@example.org")).await;
    assert!(matches!(got, Err(EmailError::Unavailable(_))), "a refused connection is an outage: {got:?}");
}

/// `starttls` is a requirement and not a preference: a relay that does not offer it is not talked to in the clear (the downgrade a network
/// attacker, or a relay that was configured without a certificate, would otherwise get for free), and neither is one that is not speaking TLS
/// when the connection is to begin with it. The sink here offers no `STARTTLS` and does not speak TLS, so a message that went through would have
/// gone in the clear: it must not, and the sink must not have been told who the message was for. The control is the same sink with `none`.
#[tokio::test]
async fn a_relay_that_cannot_do_tls_is_not_used_when_tls_is_asked_for() {
    let sink = Sink::start(Behaviour::Scripted(Script::default())).await;
    for tls in [SmtpTls::StartTls, SmtpTls::Tls] {
        let mut cfg = config(&sink);
        cfg.tls = tls;
        let got = SmtpEmail::new(&cfg).unwrap().send(&message("alice@example.org")).await;
        assert!(got.is_err(), "{tls:?}: a message went to a relay that cannot do TLS: {got:?}");
    }
    for c in sink.conversations() {
        let said = c.commands.join(" | ").to_ascii_uppercase();
        assert!(
            !said.contains("MAIL FROM") && !said.contains("RCPT TO") && !said.contains("DATA"),
            "told a relay that cannot do TLS: {said}"
        );
    }
    assert!(sink.messages().is_empty(), "NOT TLS BUT SENT: a message reached a relay that cannot do TLS");

    // CONTROL: the same sink takes a message from a client that asks for no TLS, so the refusals above were about TLS
    let mut plain = config(&sink);
    plain.tls = SmtpTls::None;
    SmtpEmail::new(&plain).unwrap().send(&message("alice@example.org")).await.expect("tls=none sends");
    assert_eq!(sink.messages().len(), 1);
}

/// RFC 5321 4.5.2 names it: a body line that is a single dot ends the message unless the client puts a second one in front of it. A pinned
/// example the specification gives, and the property below reaches it too.
#[tokio::test]
async fn a_body_line_of_one_dot_does_not_end_the_message() {
    let sink = Sink::start(Behaviour::Scripted(Script::default())).await;
    let email = SmtpEmail::new(&config(&sink)).unwrap();
    let mut m = message("alice@example.org");
    m.text = "before\n.\nafter the dot\n..two\n".into();
    email.send(&m).await.expect("accepted");
    let raw = sink.messages().remove(0);
    let parsed = parse(&raw);
    assert_eq!(squash(&parsed.get_body().unwrap()), squash(&m.text));
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap())
}

fn shared() -> &'static (Sink, SmtpEmail) {
    static S: OnceLock<(Sink, SmtpEmail)> = OnceLock::new();
    S.get_or_init(|| {
        runtime().block_on(async {
            let sink = Sink::start(Behaviour::Scripted(Script::default())).await;
            let email = SmtpEmail::new(&config(&sink)).unwrap();
            (sink, email)
        })
    })
}

/// A recipient's address, of the kind core accepts at sign-up: dot-separated atoms (every character an atom may have, the specials too), and a
/// domain of labels ending in a letters-only one. The filter is core's own check, so this is the set `is_valid_email` lets in, whatever the
/// strategy happens to make.
fn address() -> impl Strategy<Value = String> {
    let atom = "[A-Za-z0-9!#$%&'*+/=?^_`{|}~-]{1,12}";
    let local = proptest::collection::vec(atom, 1..4).prop_map(|parts| parts.join("."));
    let label = "[A-Za-z0-9]([A-Za-z0-9-]{0,10}[A-Za-z0-9])?";
    let domain = (proptest::collection::vec(label, 1..3), "[a-z]{2,6}")
        .prop_map(|(labels, tld)| format!("{}.{tld}", labels.join(".")));
    (local, domain)
        .prop_map(|(local, domain)| format!("{local}@{domain}"))
        .prop_filter("core accepts it", |a| thrust_core::is_valid_email(a))
}

/// Text that is hard on a mail: any characters, line breaks of every kind, dots at the start of lines, and long runs.
fn awkward() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>().prop_map(|s| s.chars().take(300).collect::<String>()),
        "[.a-z \\r\\n\\t]{0,200}",
        "(\\.\\r?\\n|[a-z]{1,40} ){0,12}"
    ]
}

/// A counterexample is saved beside this file, named for what it is (the default would be `tests/proptest-regressions/main.txt`, a name
/// every directory-style test binary here shares), and is committed (AGENTS.md, rule 7).
///
/// `PROPTEST_DISABLE_FAILURE_PERSISTENCE` still wins (scripts/mutants.sh sets it: a counterexample found against a deliberately broken build is
/// not a regression of the real code, and must not be saved into the tree). Naming the file here would otherwise override it.
fn config_for(cases: u32) -> ProptestConfig {
    let persistence: Option<Box<dyn proptest::test_runner::FailurePersistence>> =
        if std::env::var_os("PROPTEST_DISABLE_FAILURE_PERSISTENCE").is_some() {
            None
        } else {
            Some(Box::new(proptest::test_runner::FileFailurePersistence::Direct(
                "tests/smtp_wire/smtp_wire.proptest-regressions",
            )))
        };
    ProptestConfig { cases, failure_persistence: persistence, ..ProptestConfig::default() }
}

proptest! {
    #![proptest_config(config_for(128))]

    /// Whatever the message says, what arrives is that message: the same sender, one recipient, the subject and the text (and the HTML alternative)
    /// as given, read by a parser that is not the adapter's, under the id the adapter returned, and with no header the adapter did not mean
    /// to write. The subject is the way in for a header, so it is made of whatever a person might type, line breaks and "Bcc:" included.
    #[test]
    fn prop_a_message_arrives_as_it_was_given(to in address(), subject in awkward(), text in awkward(), html in proptest::option::of(awkward())) {
        let (sink, email) = shared();
        let given = EmailMessage { to: to.clone(), subject: subject.clone(), text: text.clone(), html: html.clone() };
        let id = runtime().block_on(email.send(&given)).expect("the sink accepts everything");

        let conversations = sink.conversations();
        let conversation = conversations
            .iter()
            .find(|c| c.data.as_deref().is_some_and(|d| parse(d).headers.iter().any(|h| h.get_key().eq_ignore_ascii_case("message-id") && h.get_value() == id)))
            .expect("the sink holds a message under the returned id");
        // the envelope: one sender, one recipient, as given
        let mails: Vec<_> = conversation.commands.iter().filter(|c| c.to_ascii_uppercase().starts_with("MAIL FROM")).collect();
        let rcpts: Vec<_> = conversation.commands.iter().filter(|c| c.to_ascii_uppercase().starts_with("RCPT TO")).collect();
        prop_assert_eq!(mails.len(), 1);
        prop_assert!(mails[0].contains("<no-reply@example.com>"), "the envelope sender: {}", mails[0]);
        prop_assert_eq!(rcpts.len(), 1);
        prop_assert!(rcpts[0].contains(&format!("<{to}>")), "the envelope recipient: {}", rcpts[0]);

        let raw = conversation.data.clone().unwrap();
        let parsed = parse(&raw);
        // the headers: the ones meant, once each, and no others
        let allowed = ["from", "to", "subject", "message-id", "date", "mime-version", "content-type", "content-transfer-encoding"];
        for h in &parsed.headers {
            let name = h.get_key().to_ascii_lowercase();
            prop_assert!(allowed.contains(&name.as_str()), "a header nobody meant: {name}");
        }
        for name in ["from", "to", "subject", "message-id", "date"] {
            let n = parsed.headers.iter().filter(|h| h.get_key().eq_ignore_ascii_case(name)).count();
            prop_assert_eq!(n, 1, "{} headers named {}", n, name);
        }
        prop_assert!(parsed.headers.get_first_value("From").unwrap().contains("no-reply@example.com"));
        prop_assert_eq!(parsed.headers.get_first_value("To").unwrap(), to);
        prop_assert_eq!(bare(&parsed.headers.get_first_value("Subject").unwrap()), bare(&subject));
        // the body
        match &html {
            None => prop_assert_eq!(squash(&parsed.get_body().unwrap()), squash(&text)),
            Some(html) => {
                prop_assert_eq!(parsed.subparts.len(), 2);
                prop_assert_eq!(squash(&parsed.subparts[0].get_body().unwrap()), squash(&text));
                prop_assert_eq!(parsed.subparts[1].ctype.mimetype.as_str(), "text/html");
                prop_assert_eq!(squash(&parsed.subparts[1].get_body().unwrap()), squash(html));
            }
        }
    }
}

/// The check above reads "no header nobody meant" off a parser, so the control is the parser being shown a message that does have one: a raw
/// message with a `Bcc` line must fail the same test. Without it, a parser that dropped unknown headers would make the property pass for any
/// adapter.
#[test]
fn the_header_check_sees_an_injected_header() {
    let raw = b"From: a@example.com\r\nTo: b@example.org\r\nSubject: hi\r\nBcc: victim@example.net\r\nMessage-ID: <1@x>\r\nDate: Mon, 1 Jan 2024 00:00:00 +0000\r\n\r\nbody\r\n";
    let parsed = parse(raw);
    let allowed =
        ["from", "to", "subject", "message-id", "date", "mime-version", "content-type", "content-transfer-encoding"];
    let stray: Vec<_> = parsed
        .headers
        .iter()
        .map(|h| h.get_key().to_ascii_lowercase())
        .filter(|n| !allowed.contains(&n.as_str()))
        .collect();
    assert_eq!(stray, vec!["bcc".to_string()]);
}

proptest! {
    #![proptest_config(config_for(64))]

    /// The port says one recipient, and the recipient is a bare address. Anything that would make it more (a list, a name with an address in
    /// angle brackets, a line break and a header or a command after the address) is refused before a connection is made, so the sink is
    /// never talked to.
    #[test]
    fn prop_a_recipient_that_is_not_one_bare_address_is_refused_and_nothing_is_sent(
        to in address(),
        other in address(),
        tail in prop_oneof![
            Just(", ".to_string()),
            Just("\r\nBcc: ".to_string()),
            Just("\nBcc: ".to_string()),
            Just(">\r\nRCPT TO:<".to_string()),
            Just(" ".to_string()),
            Just("\r\n".to_string()),
            Just(";".to_string()),
        ],
    ) {
        let sink = runtime().block_on(Sink::start(Behaviour::Scripted(Script::default())));
        let email = SmtpEmail::new(&config(&sink)).unwrap();
        for bad in [format!("{to}{tail}{other}"), format!("Name <{to}>"), format!("<{to}>"), format!("{tail}{to}"), String::new()] {
            let got = runtime().block_on(email.send(&message(&bad)));
            prop_assert!(matches!(got, Err(EmailError::Rejected(_))), "{bad:?} was not refused: {got:?}");
        }
        prop_assert!(sink.conversations().is_empty(), "a connection was made for a recipient that is not an address");
        // the control: the bare address on its own goes through, to this very sink
        runtime().block_on(email.send(&message(&to))).expect("a bare address is sent");
        prop_assert_eq!(sink.messages().len(), 1);
    }
}
