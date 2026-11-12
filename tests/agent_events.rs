use devtabs::activity::{Event, Phase, State};
fn event(turn: &str, kind: &str) -> Event {
    Event {
        transcript: None,
        provider: "codex".into(),
        conversation: "conversation".into(),
        turn: Some(turn.into()),
        kind: kind.into(),
        child: false,
        cwd: ".".into(),
    }
}
#[test]
fn stop_is_a_report_duplicate_is_ignored_and_continuation_retains_logical_generation() {
    let mut s = State::default();
    assert!(s.apply(&event("A", "UserPromptSubmit"), 1));
    let g = s.generation;
    assert!(s.apply(&event("A", "Stop"), 2));
    assert_eq!(s.phase, Phase::Reported);
    let first = s.result;
    assert!(!s.apply(&event("A", "Stop"), 3));
    assert_eq!(s.result, first);
    assert!(s.apply(&event("A", "UserPromptSubmit"), 4));
    assert_eq!(s.generation, g);
    assert!(s.apply(&event("A", "Stop"), 5));
    assert!(s.result > first);
    s.ack(first);
    assert!(s.pending());
    s.ack(s.result);
    assert!(!s.pending());
}
#[test]
fn children_old_turns_compaction_and_wrong_conversations_cannot_complete_current_work() {
    let mut s = State::default();
    s.apply(&event("A", "UserPromptSubmit"), 1);
    s.apply(&event("B", "UserPromptSubmit"), 2);
    assert!(!s.apply(&event("A", "Stop"), 3));
    let mut child = event("B", "Stop");
    child.child = true;
    assert!(!s.apply(&child, 3));
    assert!(!s.apply(&event("B", "PostCompact"), 3));
    let mut other = event("B", "Stop");
    other.conversation = "other".into();
    assert!(!s.apply(&other, 3));
    assert_eq!(s.phase, Phase::Working);
    assert!(s.apply(&event("B", "Interrupt"), 4));
    assert_eq!(s.phase, Phase::Interrupted);
}
#[test]
fn reading_latest_result_keeps_previous_unread_results_and_late_prompt_does_not_rewind_turn() {
    let mut s = State::default();
    s.apply(&event("A", "UserPromptSubmit"), 1);
    s.apply(&event("A", "Stop"), 2);
    s.apply(&event("B", "UserPromptSubmit"), 3);
    s.apply(&event("B", "Stop"), 4);
    s.ack(2);
    assert!(s.pending(), "Reading B must not silently acknowledge A");
    s.ack(1);
    assert!(!s.pending());
    assert!(!s.apply(&event("A", "UserPromptSubmit"), 5));
    assert_eq!(s.turn.as_deref(), Some("B"));
}

#[test]
fn only_accepted_provider_events_update_the_transcript_binding() {
    let mut s = State::default();
    let mut start = event("A", "UserPromptSubmit");
    start.transcript = Some("C:/sessions/own.jsonl".into());
    assert!(s.apply(&start, 1));
    assert_eq!(s.transcript, start.transcript);
    let mut child = event("A", "Stop");
    child.child = true;
    child.transcript = Some("C:/sessions/child.jsonl".into());
    assert!(!s.apply(&child, 2));
    assert_eq!(s.transcript, start.transcript);
    let mut stop = event("A", "Stop");
    stop.transcript = Some("C:/sessions/final.jsonl".into());
    assert!(s.apply(&stop, 3));
    assert_eq!(s.transcript, stop.transcript);
    let json = serde_json::to_string(&s).unwrap();
    let restored: State = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.transcript, stop.transcript);
}
