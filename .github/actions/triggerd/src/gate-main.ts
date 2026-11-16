import { verifyGate } from "./gate.js";

function value(name: string): unknown {
  const raw = process.env[name];
  if (!raw) throw new Error();
  return JSON.parse(raw);
}

try {
  process.stdout.write(`${JSON.stringify(verifyGate(value("RULES_JSON"), value("NEEDS_JSON"), value("BINDINGS_JSON")))}\n`);
} catch {
  process.stderr.write("triggerd: gate failed\n");
  process.exitCode = 1;
}
