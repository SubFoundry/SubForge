export type InlineActionState = {
  phase: "idle" | "loading" | "success" | "error";
  title: string;
  description: string;
};

export function InlineActionFeedback({ state }: { state: InlineActionState }) {
  if (state.phase === "idle") {
    return null;
  }

  return (
    <div
      className={`ui-inline-feedback ui-inline-feedback-${state.phase}`}
      role="status"
      aria-live="polite"
    >
      <p className="ui-inline-feedback-title">{state.title}</p>
      <p className="ui-inline-feedback-desc">{state.description}</p>
    </div>
  );
}

