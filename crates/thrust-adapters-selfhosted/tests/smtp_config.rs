//! The SMTP adapter's configuration (`SmtpConfig::from_lookup`, `SmtpEmail::new`): what a deployment may set, what is a mistake and is refused at
//! start, and that a password is in no message and no `Debug`. Hermetic.
//!
//! The matrix is every combination of the eight settings over a small domain each (54,000 of them), checked against a reference written another
//! way: a list of what is wrong, and an expectation of what is made when nothing is. A finite domain, so exhaustive and not sampled.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

use std::collections::HashMap;
use std::time::Duration;

use thrust_adapters_selfhosted::smtp::{SmtpConfig, SmtpEmail, SmtpTls};

const PASSWORD: &str = "S3cr3t-Value-XYZ-do-not-print";

const HOSTS: [Option<&str>; 5] =
    [None, Some(""), Some("mail.example.com"), Some("bad host"), Some("https://mail.example.com/")];
const TLSES: [Option<&str>; 6] = [None, Some("starttls"), Some("tls"), Some("none"), Some("STARTTLS"), Some("ssl")];
const PORTS: [Option<&str>; 5] = [None, Some("2525"), Some("0"), Some("65536"), Some("x")];
const USERS: [Option<&str>; 3] = [None, Some(""), Some("mailer")];
const PASSES: [Option<&str>; 3] = [None, Some(""), Some(PASSWORD)];
const FROMS: [Option<&str>; 4] =
    [None, Some("no-reply@example.com"), Some("Thrust <no-reply@example.com>"), Some("not an address")];
const CAS: [Option<&str>; 2] = [None, Some("/etc/thrust/relay-ca.pem")];
const TIMEOUTS: [Option<&str>; 5] = [None, Some("5"), Some("0"), Some("301"), Some("x")];

const HOST: &str = "THRUST_SMTP_HOST";

/// An empty value is an unset one.
fn set(v: Option<&str>) -> Option<&str> {
    v.filter(|s| !s.is_empty())
}

/// What the reference says is wrong with this combination: the variables that make it a mistake. Empty is a configuration that is fine.
fn wrong(c: &[Option<&str>; 8]) -> Vec<&'static str> {
    let [host, tls, port, user, pass, from, ca, timeout] = c.map(set);
    let mut bad = Vec::new();
    if host.is_none() {
        // nothing but the host may be set without it
        for (name, v) in [
            ("THRUST_SMTP_TLS", tls),
            ("THRUST_SMTP_PORT", port),
            ("THRUST_SMTP_USERNAME", user),
            ("THRUST_SMTP_PASSWORD", pass),
            ("THRUST_SMTP_FROM", from),
            ("THRUST_SMTP_CA_FILE", ca),
            ("THRUST_SMTP_TIMEOUT_SECS", timeout),
        ] {
            if v.is_some() {
                bad.push(name);
            }
        }
        return bad;
    }
    if host.is_some_and(|h| h.contains(' ') || h.contains('/')) {
        bad.push(HOST);
    }
    let tls_ok = tls.is_none_or(|t| ["starttls", "tls", "none"].contains(&t.to_ascii_lowercase().as_str()));
    if !tls_ok {
        bad.push("THRUST_SMTP_TLS");
    }
    if port.is_some_and(|p| p.parse::<u16>().ok().filter(|n| *n != 0).is_none()) {
        bad.push("THRUST_SMTP_PORT");
    }
    if user.is_some() != pass.is_some() {
        bad.push("THRUST_SMTP_USERNAME");
        bad.push("THRUST_SMTP_PASSWORD");
    }
    if user.is_some() && pass.is_some() && tls.is_some_and(|t| t.eq_ignore_ascii_case("none")) {
        bad.push("THRUST_SMTP_TLS");
    }
    if from.is_none_or(|f| !f.contains("no-reply@example.com")) {
        bad.push("THRUST_SMTP_FROM");
    }
    if timeout.is_some_and(|t| t.parse::<u64>().ok().filter(|n| (1..=300).contains(n)).is_none()) {
        bad.push("THRUST_SMTP_TIMEOUT_SECS");
    }
    bad
}

fn lookup(c: &[Option<&str>; 8]) -> impl Fn(&str) -> Option<String> {
    let names = [
        HOST,
        "THRUST_SMTP_TLS",
        "THRUST_SMTP_PORT",
        "THRUST_SMTP_USERNAME",
        "THRUST_SMTP_PASSWORD",
        "THRUST_SMTP_FROM",
        "THRUST_SMTP_CA_FILE",
        "THRUST_SMTP_TIMEOUT_SECS",
    ];
    let env: HashMap<String, String> =
        names.iter().zip(c.iter()).filter_map(|(n, v)| v.map(|v| (n.to_string(), v.to_string()))).collect();
    move |name: &str| env.get(name).cloned()
}

#[test]
fn every_combination_of_settings_is_made_or_refused_for_the_reason_the_reference_gives() {
    let mut cases = 0u32;
    let mut disagreements: Vec<String> = Vec::new();
    for host in HOSTS {
        for tls in TLSES {
            for port in PORTS {
                for user in USERS {
                    for pass in PASSES {
                        for from in FROMS {
                            for ca in CAS {
                                for timeout in TIMEOUTS {
                                    cases += 1;
                                    let combo = [host, tls, port, user, pass, from, ca, timeout];
                                    let got = SmtpConfig::from_lookup(lookup(&combo));
                                    let reasons = wrong(&combo);
                                    let verdict = match (&got, reasons.is_empty()) {
                                        (Err(e), false) => {
                                            let said = e.to_string();
                                            reasons.iter().any(|r| said.contains(r)).then_some(()).ok_or(format!(
                                                "refused, but the message names none of {reasons:?}: {said}"
                                            ))
                                        }
                                        (Err(e), true) => Err(format!("refused a good configuration: {e}")),
                                        (Ok(Some(_)), false) => {
                                            Err(format!("made a configuration that is wrong in {reasons:?}"))
                                        }
                                        (Ok(None), false) => {
                                            Err(format!("treated as no mail service though {reasons:?}"))
                                        }
                                        (Ok(None), true) => set(host)
                                            .is_none()
                                            .then_some(())
                                            .ok_or("no mail service with a host".to_string()),
                                        (Ok(Some(cfg)), true) => {
                                            let tls_mode = match tls.map(str::to_ascii_lowercase).as_deref() {
                                                None | Some("starttls") => SmtpTls::StartTls,
                                                Some("tls") => SmtpTls::Tls,
                                                _ => SmtpTls::None,
                                            };
                                            let default_port = match tls_mode {
                                                SmtpTls::None => 25,
                                                SmtpTls::StartTls => 587,
                                                SmtpTls::Tls => 465,
                                            };
                                            let ok = cfg.host == set(host).unwrap()
                                                && cfg.tls == tls_mode
                                                && cfg.port == set(port).map_or(default_port, |p| p.parse().unwrap())
                                                && cfg.credentials
                                                    == set(user)
                                                        .zip(set(pass))
                                                        .map(|(u, p)| (u.to_string(), p.to_string()))
                                                && cfg.ca_file.as_deref().map(|p| p.to_str().unwrap()) == set(ca)
                                                && cfg.timeout
                                                    == Duration::from_secs(
                                                        set(timeout).map_or(10, |t| t.parse().unwrap()),
                                                    )
                                                && cfg.from.email.to_string() == "no-reply@example.com";
                                            ok.then_some(()).ok_or(format!("made the wrong configuration: {cfg:?}"))
                                        }
                                    };
                                    if let Err(why) = verdict {
                                        disagreements.push(format!("{combo:?}: {why}"));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(cases, 5 * 6 * 5 * 3 * 3 * 4 * 2 * 5, "the domain is every combination");
    assert!(
        disagreements.is_empty(),
        "{} of {cases} combinations disagree with the reference, the first few: {:#?}",
        disagreements.len(),
        &disagreements[..disagreements.len().min(6)]
    );
}

/// The reference above is only worth what it can fail: a configuration that is plainly good is made, and each kind of mistake is refused (the
/// matrix covers them all; these are the control that it is not satisfied by a function that refuses everything, or accepts everything).
#[test]
fn a_good_configuration_is_made_and_a_bad_one_is_refused() {
    let good = [
        Some("mail.example.com"),
        Some("tls"),
        None,
        Some("mailer"),
        Some(PASSWORD),
        Some("no-reply@example.com"),
        None,
        None,
    ];
    assert!(matches!(SmtpConfig::from_lookup(lookup(&good)), Ok(Some(_))));
    let none = [None; 8];
    assert!(matches!(SmtpConfig::from_lookup(lookup(&none)), Ok(None)));
    let half = [None, None, Some("2525"), None, None, None, None, None];
    assert!(SmtpConfig::from_lookup(lookup(&half)).is_err());
    let clear = [
        Some("mail.example.com"),
        Some("none"),
        None,
        Some("mailer"),
        Some(PASSWORD),
        Some("no-reply@example.com"),
        None,
        None,
    ];
    assert!(SmtpConfig::from_lookup(lookup(&clear)).is_err(), "a password over a connection in the clear");
}

/// A password is a secret: it is in no error the configuration gives, however it is wrong, and `Debug` (which a log line or a panic message
/// prints) shows it as redacted and still shows the user.
#[test]
fn the_password_is_in_no_message_and_no_debug() {
    for host in HOSTS {
        for tls in TLSES {
            for port in PORTS {
                for from in FROMS {
                    let combo = [host, tls, port, Some("mailer"), Some(PASSWORD), from, None, None];
                    match SmtpConfig::from_lookup(lookup(&combo)) {
                        Err(e) => assert!(!e.to_string().contains(PASSWORD), "{combo:?}: {e}"),
                        Ok(Some(cfg)) => {
                            let shown = format!("{cfg:?}");
                            assert!(!shown.contains(PASSWORD), "{shown}");
                            assert!(shown.contains("<redacted>") && shown.contains("mailer"), "{shown}");
                        }
                        Ok(None) => {}
                    }
                }
            }
        }
    }
}

fn base(ca_file: Option<std::path::PathBuf>) -> SmtpConfig {
    SmtpConfig {
        host: "mail.example.com".into(),
        port: 587,
        tls: SmtpTls::StartTls,
        credentials: None,
        from: "no-reply@example.com".parse().unwrap(),
        ca_file,
        timeout: Duration::from_secs(10),
    }
}

/// The CA file is read when the server starts, so a typo is an error at start and not at the first password reset: a path that is not a file, an
/// empty file and a file that is not PEM are each refused by name, and a certificate is taken.
#[test]
fn a_ca_file_that_is_missing_empty_or_not_a_certificate_is_refused_at_start() {
    let dir = std::env::temp_dir().join(format!("thrust-smtp-config-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, body: &str| {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    };
    let refused = |path: std::path::PathBuf| SmtpEmail::new(&base(Some(path))).unwrap_err().to_string();

    assert!(refused(dir.join("missing.pem")).contains("THRUST_SMTP_CA_FILE"), "a path that is not a file");
    assert!(refused(write("empty.pem", "")).contains("no certificate"), "an empty file");
    assert!(refused(write("text.pem", "this is not a certificate\n")).contains("no certificate"), "a text file");
    assert!(
        refused(write("key.pem", "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIA==\n-----END PRIVATE KEY-----\n"))
            .contains("no certificate"),
        "a key"
    );

    // the control: a real certificate (made here) is taken, so the refusals above are about the files
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let cert = params.self_signed(&rcgen::KeyPair::generate().unwrap()).unwrap();
    let good = write("ca.pem", &cert.pem());
    SmtpEmail::new(&base(Some(good))).expect("a PEM certificate is taken");
    // and a configuration with no TLS never reads the file it names
    let mut plain = base(Some(dir.join("missing.pem")));
    plain.tls = SmtpTls::None;
    SmtpEmail::new(&plain).expect("tls=none has no use for a CA file");

    std::fs::remove_dir_all(&dir).unwrap();
}
