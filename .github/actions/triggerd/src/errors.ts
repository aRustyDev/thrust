import type { TriggerdErrorCode } from "./types.js";

export class TriggerdError extends Error {
  readonly code: TriggerdErrorCode;

  constructor(code: TriggerdErrorCode, message: string) {
    super(message);
    this.name = "TriggerdError";
    this.code = code;
  }
}
