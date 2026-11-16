export type TriggerdErrorCode =
  | "CONFIG_INVALID"
  | "LIMIT_EXCEEDED"
  | "INPUT_INVALID"
  | "SOURCE_INVALID"
  | "SOURCE_STALE"
  | "AUTH_DENIED"
  | "SOURCE_INCOMPLETE"
  | "SOURCE_UNAVAILABLE"
  | "DEADLINE_EXCEEDED"
  | "OUTPUT_INVALID";

export type CompleteChangeSet = { readonly kind: "complete"; readonly count: number };
export type NotCollectedChangeSet = {
  readonly kind: "not-collected";
  readonly reason: "explicit-full-check" | "manual" | "schedule" | "new-branch";
};

export type ChangeSet = CompleteChangeSet | NotCollectedChangeSet;
