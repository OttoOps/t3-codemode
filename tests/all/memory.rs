use crate::common::*;
use serde_json::json;

#[tokio::test]
async fn mem_round_trips_in_the_session_namespace() {
    let h = H::new();
    let out = h
        .exec_as(WORKER, "async () => { await mem.set('a', { n: 1 }); await mem.set('b', [1, 2]); await mem.set('x', 'y'); return [await mem.get('a'), await mem.list(), await mem.list('a'), await mem.del('x'), await mem.del('x'), await mem.get('x')] }")
        .await;
    assert_eq!(result(&out), json!([{"n": 1}, ["a", "b", "x"], ["a"], true, false, null]));
    let other = h.exec_as(TICKET, "async () => mem.get('a')").await;
    assert_eq!(result(&other), json!(null));
    let shared = h.exec_as(TICKET, "async () => mem.ns('thread:mcp:worker').get('a')").await;
    assert_eq!(result(&shared), json!({"n": 1}));
}

#[tokio::test]
async fn named_namespaces_are_shared_across_threads() {
    let h = H::new();
    result(&h.exec_as(WORKER, "async () => mem.ns('work:widgets#41').set('status', 'green')").await);
    let out = h.exec_as(BOSS, "async () => mem.ns('work:widgets#41').get('status')").await;
    assert_eq!(result(&out), json!("green"));
}

#[tokio::test]
async fn mem_rejects_bad_input() {
    let h = H::new();
    assert!(err_text(&h.exec("async () => mem.set('k', undefined)").await).contains("not undefined"));
    assert!(err_text(&h.exec("async () => mem.set('', 1)").await).contains("non-empty"));
    assert!(err_text(&h.exec("async () => mem.set('__vars__', 1)").await).contains("reserved"));
    assert!(err_text(&h.exec("async () => mem.set('big', 'x'.repeat(1100000))").await).contains("over 1 MB"));
}

#[tokio::test]
async fn vars_persist_across_executes_and_a_restart() {
    let mut h = H::new();
    let bump = "async () => { vars.count = (vars.count ?? 0) + 1; vars.seen = [...(vars.seen ?? []), vars.count]; return vars.count }";
    assert_eq!(result(&h.exec_as(WORKER, bump).await), json!(1));
    assert_eq!(result(&h.exec_as(WORKER, bump).await), json!(2));
    assert_eq!(result(&h.exec_as(TICKET, bump).await), json!(1));
    result(&h.exec_as(WORKER, "async () => mem.set('kept', 'yes')").await);
    h.reopen();
    assert_eq!(result(&h.exec_as(WORKER, bump).await), json!(3));
    assert_eq!(
        result(&h.exec_as(WORKER, "async () => [vars.seen, await mem.get('kept')]").await),
        json!([[1, 2, 3], "yes"])
    );
}

#[tokio::test]
async fn an_explicit_session_names_the_namespace() {
    let h = H::new();
    let run = |session: &'static str| {
        h.app.execute(t3_codemode::app::ExecuteRequest {
            code: "async () => { vars.n = (vars.n ?? 0) + 1; return vars.n }".into(),
            session: Some(session.into()),
            timeout_ms: None,
            caller: Some(WORKER.into()),
            caller_from_arg: false,
            reported: Default::default(),
        })
    };
    assert_eq!(result(&run("s1").await), json!(1));
    assert_eq!(result(&run("s1").await), json!(2));
    assert_eq!(result(&run("s2").await), json!(1));
}

#[tokio::test]
async fn vars_survive_a_thrown_error_but_not_a_cpu_kill() {
    let h = H::new();
    let out = h.exec("async () => { vars.a = 1; throw new Error('late') }").await;
    assert!(out.is_error);
    assert_eq!(result(&h.exec("async () => vars.a").await), json!(1));
    let out = h.exec("async () => { vars.a = 2; while (true) {} }").await;
    assert!(out.is_error);
    assert_eq!(result(&h.exec("async () => vars.a").await), json!(1));
}
