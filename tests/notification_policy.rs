use devtabs::notifications::Policy;
#[test]
fn global_off_wins_and_permission_does_not_consume_single_end_subscription() {
    let mut p = Policy {
        muted: true,
        once: true,
        ..Default::default()
    };
    assert!(!p.allow(false, true));
    assert!(p.once);
    assert!(!p.allow(true, false));
    assert!(p.once);
    assert!(p.allow(true, true));
    assert!(!p.once);
    assert!(!p.allow(true, true));
}
#[test]
fn snooze_consumes_once_and_never_replays_after_read_new_generation_or_mute() {
    let mut p = Policy {
        snooze: Some((300, 2, 1)),
        ..Default::default()
    };
    assert!(!p.due(299, 2, true, 1));
    assert!(p.due(300, 2, true, 1));
    assert!(!p.due(301, 2, true, 1));
    for (result, read, generation, muted) in [
        (2, false, 1, false),
        (3, true, 1, false),
        (2, true, 2, false),
        (2, true, 1, true),
    ] {
        p.snooze = Some((300, 2, 1));
        p.muted = muted;
        assert!(!p.due(300, result, read, generation));
        assert!(p.snooze.is_none());
    }
}
