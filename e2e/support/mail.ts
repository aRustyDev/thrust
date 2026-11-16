// The mail a Worker was asked to send, read from outside it: the journeys that need a code a person is mailed (e2e/journeys-mail/) read it here.
//
// A Worker run by `wrangler dev` (local, no `remote = true`) has its `send_email` binding simulated: nothing is sent, and the simulator writes what
// it was asked to send to the dev server's log (the sender, the recipient, the subject, and the paths of the files that hold the text and the
// html). scripts/contract.sh's `mail` and `e2e-mail` passes start the Worker into a log of their own and export its path as
// THRUST_MAIL_WRANGLER_LOG. The Rust suite reads the same log the same way (crates/testing/suites/api/tests/mail.rs, `simulated_mails`); this is
// that reader's small cousin: it needs the recipient and the subject, and the subject of a sign-in code is `Your Thrust sign-in code is NNNNNN`
// (crates/core/usecases/src/login_code.rs), so the code is read from it and the body is not opened.
//
// What the log looks like (wrangler 4.x, colours stripped):
//
//   [wrangler:info] send_email binding called with MessageBuilder:      <- a line that holds this text starts a message
//   From: Thrust <no-reply@thrust.test>
//   To: someone@example.test
//   Subject: Your Thrust sign-in code is 123456
//
//   Text: /…/email-text/<id>.txt
//
// The simulator's lines reach the log a few milliseconds AFTER the HTTP response that caused them (that is why the Rust suite makes a marker
// mail and waits for it), so a journey never reads the log once: `waitForMail` reads it again, every 100 ms, until the message it needs is there.
import { readFileSync } from "node:fs";

export type Mail = { from: string; to: string; subject: string };

// wrangler colours its announce line even under NO_COLOR
const COLOUR = /\u001b\[[0-9;]*m/g;
const ANNOUNCE = "send_email binding called with MessageBuilder";

export function logPath(): string {
  const path = process.env.THRUST_MAIL_WRANGLER_LOG;
  if (!path) throw new Error("THRUST_MAIL_WRANGLER_LOG is not set: the target was not started by scripts/contract.sh's e2e-mail pass");
  return path;
}

// Every message the log holds, in the order it was written. Pure: the text of a log in, the messages out.
export function mailsIn(log: string): Mail[] {
  const lines = log.replace(COLOUR, "").split(/\r?\n/);
  const found: Mail[] = [];
  lines.forEach((line, i) => {
    if (!line.includes(ANNOUNCE)) return;
    const mail: Mail = { from: "", to: "", subject: "" };
    for (const next of lines.slice(i + 1)) {
      // the message's lines end where the next log line (`[wrangler:info] ...`) or the next message begins
      if (next.startsWith("[") || next.includes(ANNOUNCE)) break;
      if (next.startsWith("From: ")) mail.from = next.slice("From: ".length).trim();
      else if (next.startsWith("To: ")) mail.to = next.slice("To: ".length).trim();
      else if (next.startsWith("Subject: ")) mail.subject = next.slice("Subject: ".length).trim();
    }
    found.push(mail);
  });
  return found;
}

// The messages mailed to one address, oldest first. A log that is not there yet (the Worker has not written to it) is no mail.
export function mailsTo(address: string): Mail[] {
  let log: string;
  try {
    log = readFileSync(logPath(), "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return [];
    throw error;
  }
  const wanted = address.toLowerCase();
  return mailsIn(log).filter((m) => m.to.toLowerCase() === wanted);
}

// Wait until `count` messages have been mailed to `address`, and give them: the log lags the response, so the first look finds nothing.
export async function waitForMail(address: string, count: number, timeoutMs: number): Promise<Mail[]> {
  const deadline = Date.now() + timeoutMs;
  let have = mailsTo(address);
  while (have.length < count && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 100));
    have = mailsTo(address);
  }
  if (have.length < count) {
    throw new Error(
      `the mail never reached ${logPath()}: ${have.length} message(s) to ${address} in ${Math.round(timeoutMs / 1000)} s, and ${count} wanted`,
    );
  }
  return have;
}

// The six digits of a sign-in code mail's subject. Strict on purpose: a subject of another shape is a mail the journey did not expect.
export function codeIn(mail: Mail): string {
  const found = /^Your Thrust sign-in code is (\d{6})$/.exec(mail.subject);
  if (!found?.[1]) throw new Error(`the subject of the mail is not a sign-in code's: '${mail.subject}'`);
  return found[1];
}
