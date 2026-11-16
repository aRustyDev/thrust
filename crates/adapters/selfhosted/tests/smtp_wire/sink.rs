//! A scripted SMTP server on `127.0.0.1:0`, in the test's own process: it says what it is told to at each step of a conversation, and keeps
//! what the client said. It exists for what a real server will not do on demand (answer `451` after the recipient, answer nothing at all,
//! hang up) and for what a test can then count (connections, commands, the bytes of a message). That the adapter also works against a
//! server nobody here wrote is `smtp_sink`'s (Mailpit's) to show.
//!
//! It speaks just enough RFC 5321 for `lettre` to send one message without TLS: `EHLO`, `MAIL`, `RCPT`, `DATA` with its dot-stuffing, `RSET`,
//! `NOOP`, `QUIT`. A client that asks for more (`STARTTLS`, `AUTH`) is told `502`.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// A reply: a three-digit code and its text.
#[derive(Clone, Debug)]
pub struct Reply {
    pub code: u16,
    pub text: String,
}

impl Reply {
    pub fn new(code: u16, text: &str) -> Self {
        Reply { code, text: text.to_string() }
    }
    fn line(&self) -> String {
        format!("{} {}\r\n", self.code, self.text)
    }
}

/// What the server answers at each step. Every field defaults to what a server that takes the message answers.
#[derive(Clone, Debug)]
pub struct Script {
    pub greeting: Reply,
    pub mail: Reply,
    pub rcpt: Reply,
    /// The answer to `DATA`. `354` invites the body; anything else refuses it and no body is read.
    pub data: Reply,
    /// The answer once the body has been read.
    pub end: Reply,
}

impl Default for Script {
    fn default() -> Self {
        Script {
            greeting: Reply::new(220, "wire.test ESMTP"),
            mail: Reply::new(250, "sender ok"),
            rcpt: Reply::new(250, "recipient ok"),
            data: Reply::new(354, "go ahead"),
            end: Reply::new(250, "queued"),
        }
    }
}

/// What the server does with a connection.
#[derive(Clone, Debug)]
pub enum Behaviour {
    /// Talks, as scripted.
    Scripted(Script),
    /// Accepts the connection and never says a word (a relay that is up and does not answer).
    Silent,
    /// Accepts the connection and closes it at once.
    HangUp,
}

/// What one connection saw.
#[derive(Clone, Debug, Default)]
pub struct Conversation {
    /// Every command line, as sent (`MAIL FROM:<a@b>`, `RCPT TO:<c@d>`, `DATA`...).
    pub commands: Vec<String>,
    /// The body of the message, dot-unstuffed, when the server took it.
    pub data: Option<Vec<u8>>,
}

pub struct Sink {
    pub addr: SocketAddr,
    seen: Arc<Mutex<Vec<Conversation>>>,
}

impl Sink {
    pub async fn start(behaviour: Behaviour) -> Sink {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind a loopback port");
        let addr = listener.local_addr().expect("its address");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let kept = seen.clone();
        tokio::spawn(async move {
            // Held open for the life of the test: a silent connection that is dropped is a hang-up, not silence.
            let mut quiet: Vec<TcpStream> = Vec::new();
            loop {
                let Ok((stream, _)) = listener.accept().await else { return };
                match behaviour.clone() {
                    Behaviour::Scripted(script) => {
                        let kept = kept.clone();
                        tokio::spawn(async move { converse(stream, script, kept).await });
                    }
                    Behaviour::Silent => quiet.push(stream),
                    Behaviour::HangUp => drop(stream),
                }
            }
        });
        Sink { addr, seen }
    }

    pub fn host(&self) -> String {
        self.addr.ip().to_string()
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Every conversation so far, in the order they began.
    pub fn conversations(&self) -> Vec<Conversation> {
        self.seen.lock().expect("the sink's log").clone()
    }

    /// The bodies of the messages taken so far.
    pub fn messages(&self) -> Vec<Vec<u8>> {
        self.conversations().into_iter().filter_map(|c| c.data).collect()
    }
}

async fn converse(stream: TcpStream, script: Script, seen: Arc<Mutex<Vec<Conversation>>>) {
    let index = {
        let mut log = seen.lock().expect("the sink's log");
        log.push(Conversation::default());
        log.len() - 1
    };
    let note =
        |f: &dyn Fn(&mut Conversation)| f(seen.lock().expect("the sink's log").get_mut(index).expect("its slot"));
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    if write.write_all(script.greeting.line().as_bytes()).await.is_err() || script.greeting.code >= 400 {
        return;
    }
    let mut line: Vec<u8> = Vec::new();
    loop {
        line.clear();
        match read.read_until(b'\n', &mut line).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let command = String::from_utf8_lossy(&line).trim_end().to_string();
        note(&|c| c.commands.push(command.clone()));
        let verb = command.split_whitespace().next().unwrap_or("").to_ascii_uppercase();
        let answer = match verb.as_str() {
            "EHLO" => "250-wire.test\r\n250 8BITMIME\r\n".to_string(),
            "HELO" => "250 wire.test\r\n".to_string(),
            "MAIL" => script.mail.line(),
            "RCPT" => script.rcpt.line(),
            "RSET" | "NOOP" => "250 ok\r\n".to_string(),
            "QUIT" => {
                let _ = write.write_all(b"221 bye\r\n").await;
                return;
            }
            "DATA" => {
                if write.write_all(script.data.line().as_bytes()).await.is_err() {
                    return;
                }
                if script.data.code != 354 {
                    continue;
                }
                let mut body: Vec<u8> = Vec::new();
                loop {
                    line.clear();
                    match read.read_until(b'\n', &mut line).await {
                        Ok(0) | Err(_) => return,
                        Ok(_) => {}
                    }
                    if line == b".\r\n" {
                        break;
                    }
                    // RFC 5321 4.5.2: a line that begins with a dot had one more put in front of it
                    body.extend_from_slice(line.strip_prefix(b".").unwrap_or(&line));
                }
                note(&|c| c.data = Some(body.clone()));
                script.end.line()
            }
            _ => "502 not implemented\r\n".to_string(),
        };
        if write.write_all(answer.as_bytes()).await.is_err() {
            return;
        }
    }
}
