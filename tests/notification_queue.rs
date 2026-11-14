use devtabs::notifications::{Notice, Queue, Target};
fn notice(task: &str, result: u64, urgent: bool) -> Notice {
    Notice {
        target: Target {
            workspace: "w".into(),
            task: task.into(),
            session: task.into(),
            run: task.into(),
            result,
        },
        title: "DevTabs".into(),
        body: "Reported".into(),
        test: false,
        category: String::new(),
        ending: !urgent,
        urgent,
        silent: true,
    }
}
#[test]
fn delivery_rechecks_current_preference_and_explicit_test_bypasses_policy() {
    let mut n = notice("task", 1, false);
    n.category = "end".into();
    let mut p = devtabs::settings::model::Notifications::default();
    assert!(devtabs::notifications::delivery_policy(&mut n, &p));
    p.enabled = Some(false);
    assert!(!devtabs::notifications::delivery_policy(&mut n, &p));
    p.enabled = Some(true);
    p.sound = Some(false);
    assert!(devtabs::notifications::delivery_policy(&mut n, &p));
    assert!(n.silent);
    p.categories.insert("end".into(), false);
    assert!(!devtabs::notifications::delivery_policy(&mut n, &p));
    n.test = true;
    n.silent = false;
    assert!(devtabs::notifications::delivery_policy(&mut n, &p));
    assert!(!n.silent);
}
#[test]
fn urgent_authorization_displaces_ordinary_burst_and_results_merge_per_task_for_one_second() {
    let mut q = Queue::default();
    for i in 0..32 {
        q.push(0, notice(&format!("t{i}"), 1, false)).unwrap();
    }
    q.push(1, notice("authorization", 0, true)).unwrap();
    assert_eq!(q.pop(1).unwrap().target.task, "authorization");
    assert!(q.pop(999).is_none());
    assert!(q.pop(1000).is_some());
    let mut q = Queue::default();
    q.push(0, notice("a", 1, false)).unwrap();
    q.push(900, notice("a", 2, false)).unwrap();
    assert!(q.pop(999).is_none());
    assert_eq!(q.pop(1000).unwrap().target.result, 2);
    assert!(q.pop(2000).is_none());
}
#[test]
fn unique_authorization_overflow_is_explicit_and_never_silently_overwritten() {
    let mut q = Queue::default();
    for i in 0..32 {
        q.push(0, notice(&format!("a{i}"), 0, true)).unwrap();
    }
    assert!(q.push(0, notice("extra", 0, true)).is_err());
    for i in 0..32 {
        assert_eq!(q.pop(0).unwrap().target.task, format!("a{i}"));
    }
}
