use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::config::Config;
use t3_codemode::fake::{check_run, github_pr, project, thread};

const REPO: &str = "acme/widgets";
const HEAD: &str = "1111111111111111111111111111111111111111";
const MOVED: &str = "2222222222222222222222222222222222222222";
const GOOD: &str = "## Summary\n\n```text\nwf.pr\n```\n\n## Evidence\n\n- **Before:** none\n  **After:** green\n\n\
## Merge Danger\n\n**Door:** two-way\n\n**Blast Radius:** board\n\nRefs #41\n";

fn setup(steps: Value) -> H {
    prepare(H::new(), steps)
}

fn prepare(h: H, steps: Value) -> H {
    let home = h.dir.path().join("home");
    let repo = h.dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    for skill in ["deploy", "publish"] {
        let d = home.join(".claude/skills").join(skill);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), format!("---\nname: {skill}\ndescription: test\n---\n\nBody.\n")).unwrap();
    }
    let p = home.join(".config/t3-workflow/config.json");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, json!({"delivery": {"steps": steps}}).to_string()).unwrap();
    h.fake.set_projects(vec![project("proj:widgets", &repo.to_string_lossy())]);
    let mut th = thread(WORKER, "idle", T0);
    th["projectId"] = json!("proj:widgets");
    h.fake.put(th);
    h
}

fn merge_only() -> Value {
    json!([{"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"}])
}

fn merge_and_deploy() -> Value {
    json!([
        {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
        {"id": "deploy", "skill": "deploy", "gate": "chain", "scope": "pr"}
    ])
}

async fn ticket_and_worker(h: &H, part: &str, method: Option<&str>) {
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    let mut pr = json!({"format": "pr"});
    if let Some(m) = method {
        pr["method"] = json!(m);
    }
    let label = format!("#41 PR {part}");
    h.wf(
        TICKET,
        "openChild",
        json!({"role": "worker", "label": label, "issue": "acme/widgets#41", "part": part, "pr": pr}),
    )
    .await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": label, "role": "worker"})).await;
}

fn open_pr(h: &H, queue: bool) {
    let mut pr = github_pr(HEAD, "CLEAN", queue);
    pr["body"] = json!(GOOD);
    pr["files"] = json!({"pageInfo": {"hasNextPage": false}, "nodes": [{"path": "src/lib.rs"}]});
    h.github.set_pr(REPO, 7, pr);
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS")]));
}

async fn ready_card(h: &H) -> String {
    let r = h.wf(WORKER, "pr", json!({"event": "ready", "repo": REPO, "pr": 7, "head": HEAD})).await;
    assert_eq!(r["status"], "gated", "{r}");
    r["card"].as_str().unwrap().to_string()
}

fn to(h: &H, thread: &str) -> Vec<String> {
    h.sent().into_iter().filter(|s| s["threadId"] == thread).map(|s| s["message"].as_str().unwrap().into()).collect()
}

fn cards(h: &H) -> Vec<Value> {
    h.app.wf.cards().unwrap().into_iter().map(|c| serde_json::to_value(c).unwrap()).collect()
}

fn mutations(h: &H) -> Vec<String> {
    h.github.operations().into_iter().filter(|op| op != "read" && op != "reconcile").collect()
}

fn merge_methods(h: &H) -> Vec<String> {
    let calls = h.github.calls();
    let merges = calls.iter().filter(|c| c["operation"] == "mergePullRequest");
    merges.map(|c| c["variables"]["method"].as_str().unwrap_or_default().to_string()).collect()
}

fn gate_of(h: &H, child: &str) -> Option<String> {
    h.app.store.link_get(child).unwrap().unwrap().gate
}

async fn digest(h: &H) -> Vec<String> {
    h.wf(WORKER, "send", json!({"kind": "event", "body": "wake the ticket"})).await;
    let woke = to(h, TICKET);
    let last = woke.last().expect("a ticket wake");
    let notes = last.split_once("Notes since your last wake").map(|(_, d)| d).unwrap_or_default();
    notes.lines().filter(|l| l.starts_with("- handled for you")).map(String::from).collect()
}

fn handoff(commit: &str) -> String {
    format!("Merged #7 at {commit}. Hand off.")
}

fn merge_commit(h: &H) -> String {
    h.github.pr(REPO, 7)["mergeCommit"]["oid"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_merge_answer_merges_in_code_and_sends_the_worker_its_handoff() {
    let h = setup(merge_only());
    ticket_and_worker(&h, "1/2", Some("rebase")).await;
    open_pr(&h, false);
    let card = ready_card(&h).await;

    let out = h.app.wf.answer(&card, "merge").await.unwrap();

    assert_eq!(out["merge"]["status"], "merged", "{out}");
    assert_eq!(mutations(&h), ["mergePullRequest", "deleteRef"]);
    assert_eq!(merge_methods(&h), ["REBASE"]);
    assert!(h.github.pr(REPO, 7)["headRef"].is_null(), "the remote branch is deleted");
    let commit = merge_commit(&h);
    let worker = to(&h, WORKER);
    assert_eq!(worker.len(), 1, "{worker:?}");
    assert!(worker[0].starts_with("[t3cm reply from=t3cm msg="), "{}", worker[0]);
    assert!(worker[0].contains(&handoff(&commit)), "{}", worker[0]);
    assert!(to(&h, TICKET).is_empty(), "a merge-only answer takes no ticket turn");
    assert!(cards(&h).is_empty(), "{:?}", cards(&h));
    assert_eq!(gate_of(&h, TICKET), None);
    assert_eq!(gate_of(&h, WORKER), None);

    h.app.wf.reconcile().await.unwrap();
    assert!(to(&h, TICKET).is_empty(), "the reconcile does not wake the ticket for the merge again");
    assert_eq!(to(&h, WORKER).len(), 1, "the worker hears of the merge once");

    let handled = digest(&h).await;
    assert!(
        handled.iter().any(|l| l.contains(&format!("the board's answer \"merge\" on card {card} went to t3cm"))),
        "{handled:?}"
    );
    let merged = handled.iter().find(|l| l.contains("merged at")).expect("a merged line");
    assert!(merged.contains("deleted branch feat/7"), "{merged}");
    assert!(merged.contains("sent #41 PR 1/2"), "{merged}");
}

#[tokio::test]
async fn a_chained_answer_merges_in_code_and_wakes_the_ticket_once_to_run_the_stages() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some("rebase")).await;
    open_pr(&h, false);
    let card = ready_card(&h).await;

    let out = h.app.wf.answer(&card, "merge and deploy").await.unwrap();

    assert_eq!(out["merge"]["status"], "merged", "{out}");
    let commit = merge_commit(&h);
    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 1, "{woke:?}");
    assert!(woke[0].starts_with("[t3cm event from=t3cm msg="), "{}", woke[0]);
    assert!(
        woke[0].contains(&format!(
            "https://github.com/acme/widgets/pull/7 merged at {} on the user's \"merge and deploy\"",
            &commit[..7]
        )),
        "{}",
        woke[0]
    );
    assert!(woke[0].contains("Remaining delivery stages for this PR, in order: deploy (pr)."), "{}", woke[0]);
    assert!(woke[0].contains("The user's answer approved deploy."), "{}", woke[0]);
    assert!(woke[0].contains("has not told #41 PR 1/2"), "{}", woke[0]);
    assert!(to(&h, WORKER).is_empty(), "the worker waits for the deploy");

    h.app.wf.reconcile().await.unwrap();
    assert_eq!(to(&h, TICKET).len(), 1, "one ticket wake per merge");
}

#[tokio::test]
async fn merge_alone_with_a_pr_stage_left_wakes_the_ticket_to_gate_it() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some("squash")).await;
    open_pr(&h, false);
    let card = ready_card(&h).await;

    h.app.wf.answer(&card, "merge").await.unwrap();

    assert_eq!(merge_methods(&h), ["SQUASH"]);
    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 1, "{woke:?}");
    assert!(woke[0].contains("The user's answer approved none of them."), "{}", woke[0]);
    assert!(to(&h, WORKER).is_empty());
}

#[tokio::test]
async fn the_last_pr_tells_the_worker_and_wakes_the_ticket_for_its_ticket_stages() {
    let h = setup(json!([
        {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
        {"id": "publish", "skill": "publish", "gate": "chain", "scope": "ticket"}
    ]));
    ticket_and_worker(&h, "2/2", Some("rebase")).await;
    open_pr(&h, false);
    let card = ready_card(&h).await;
    assert_eq!(cards(&h)[0]["options"], json!(["merge and publish", "merge"]));

    h.app.wf.answer(&card, "merge and publish").await.unwrap();

    let commit = merge_commit(&h);
    assert!(to(&h, WORKER).iter().any(|m| m.contains(&handoff(&commit))), "{:?}", to(&h, WORKER));
    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 1, "{woke:?}");
    assert!(woke[0].contains("in order: publish (ticket). The user's answer approved publish."), "{}", woke[0]);
}

#[tokio::test]
async fn an_earlier_pr_does_not_wake_the_ticket_for_ticket_stages() {
    let h = setup(json!([
        {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
        {"id": "publish", "skill": "publish", "gate": "chain", "scope": "ticket"},
        {"id": "announce", "skill": "publish", "gate": "always", "scope": "post-handoff"}
    ]));
    ticket_and_worker(&h, "1/2", Some("rebase")).await;
    open_pr(&h, false);
    let card = ready_card(&h).await;

    h.app.wf.answer(&card, "merge").await.unwrap();

    assert!(to(&h, TICKET).is_empty(), "{:?}", to(&h, TICKET));
    assert!(to(&h, WORKER).iter().any(|m| m.contains("Hand off.")));
}

#[tokio::test]
async fn a_pending_check_holds_the_approval_until_it_passes_then_merges_with_no_ticket_turn() {
    let h = setup(merge_only());
    ticket_and_worker(&h, "1/2", Some("rebase")).await;
    open_pr(&h, false);
    let card = ready_card(&h).await;
    let running = json!({"__typename": "CheckRun", "name": "CodeRabbit", "status": "IN_PROGRESS", "conclusion": null});
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS"), running]));
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "UNSTABLE"}));

    let out = h.app.wf.answer(&card, "merge").await.unwrap();

    assert_eq!(out["merge"]["status"], "waiting", "{out}");
    assert!(to(&h, TICKET).is_empty(), "the ticket starts no merge watch");
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("merge"), "the ticket is not nudged while t3cm waits");
    h.app.wf.reconcile().await.unwrap();
    assert!(mutations(&h).is_empty(), "nothing merges while a check runs: {:?}", mutations(&h));

    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS"), check_run("CodeRabbit", "SUCCESS")]));
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "CLEAN"}));
    h.app.wf.reconcile().await.unwrap();
    h.app.wf.reconcile().await.unwrap();

    assert_eq!(mutations(&h), ["mergePullRequest", "deleteRef"]);
    assert_eq!(merge_methods(&h), ["REBASE"]);
    let commit = merge_commit(&h);
    assert!(to(&h, WORKER).iter().any(|m| m.contains(&handoff(&commit))), "{:?}", to(&h, WORKER));
    assert!(to(&h, TICKET).is_empty(), "{:?}", to(&h, TICKET));
    assert_eq!(gate_of(&h, TICKET), None);
}

#[tokio::test]
async fn a_queued_answer_hands_off_once_the_queue_merges_it() {
    let h = setup(merge_only());
    ticket_and_worker(&h, "1/2", Some("merge")).await;
    open_pr(&h, true);
    let card = ready_card(&h).await;

    let out = h.app.wf.answer(&card, "merge").await.unwrap();

    assert_eq!(out["merge"]["status"], "queued", "{out}");
    assert_eq!(mutations(&h), ["enqueuePullRequest"]);
    h.app.wf.reconcile().await.unwrap();
    assert!(to(&h, WORKER).is_empty());

    h.github.update_pr(
        REPO,
        7,
        json!({"state": "MERGED", "isInMergeQueue": false, "mergeCommit": {"oid": MOVED}, "mergedAt": at(5)}),
    );
    h.app.wf.reconcile().await.unwrap();

    assert!(to(&h, WORKER).iter().any(|m| m.contains(&handoff(MOVED))), "{:?}", to(&h, WORKER));
    assert!(to(&h, TICKET).is_empty(), "{:?}", to(&h, TICKET));
    assert_eq!(mutations(&h), ["enqueuePullRequest", "deleteRef"]);
}

#[tokio::test]
async fn a_requeue_answer_queues_the_pr_again_in_code() {
    let h = setup(merge_only());
    ticket_and_worker(&h, "1/2", Some("merge")).await;
    open_pr(&h, true);
    let card = ready_card(&h).await;
    h.app.wf.answer(&card, "merge").await.unwrap();
    h.app.wf.reconcile().await.unwrap();
    h.clock.set(&at(3));
    h.github.drop_from_queue(REPO, 7, "failed_checks", &at(2));
    h.app.wf.reconcile().await.unwrap();
    let dropped = cards(&h);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0]["options"], json!(["requeue", "hold"]));

    let out = h.app.wf.answer(dropped[0]["id"].as_str().unwrap(), "requeue").await.unwrap();

    assert_eq!(out["merge"]["status"], "queued", "{out}");
    assert_eq!(mutations(&h), ["enqueuePullRequest", "enqueuePullRequest"]);
    assert!(to(&h, TICKET).is_empty(), "{:?}", to(&h, TICKET));
    assert!(cards(&h).is_empty());
}

#[tokio::test]
async fn an_ok_answer_accepts_the_queued_head_and_keeps_waiting() {
    let h = setup(merge_only());
    ticket_and_worker(&h, "1/2", Some("merge")).await;
    open_pr(&h, true);
    let card = ready_card(&h).await;
    h.app.wf.answer(&card, "merge").await.unwrap();
    h.app.wf.reconcile().await.unwrap();
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS")]));
    h.app.wf.reconcile().await.unwrap();
    let moved = cards(&h);
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert_eq!(moved[0]["options"], json!(["ok"]));

    let out = h.app.wf.answer(moved[0]["id"].as_str().unwrap(), "ok").await.unwrap();

    assert_eq!(out["accepted"], MOVED, "{out}");
    assert!(to(&h, TICKET).is_empty(), "{:?}", to(&h, TICKET));
    h.github.update_pr(
        REPO,
        7,
        json!({"state": "MERGED", "isInMergeQueue": false, "mergeCommit": {"oid": HEAD}, "mergedAt": at(5)}),
    );
    h.app.wf.reconcile().await.unwrap();
    assert!(to(&h, WORKER).iter().any(|m| m.contains(&handoff(HEAD))), "{:?}", to(&h, WORKER));
    assert!(to(&h, TICKET).is_empty(), "{:?}", to(&h, TICKET));
}

#[tokio::test]
async fn a_card_blocked_by_a_change_request_waits_for_an_admin_merge_on_github() {
    let h = setup(merge_only());
    ticket_and_worker(&h, "1/2", Some("rebase")).await;
    open_pr(&h, false);
    h.github.update_pr(
        REPO,
        7,
        json!({"mergeStateStatus": "BLOCKED", "reviewDecision": "CHANGES_REQUESTED",
               "latestOpinionatedReviews": {"nodes": [{"state": "CHANGES_REQUESTED",
                   "author": {"login": "coderabbitai"}, "commit": {"oid": HEAD}}]}}),
    );
    let card = ready_card(&h).await;
    let text = cards(&h)[0]["text"].as_str().unwrap().to_string();
    assert!(text.contains("an admin can merge it over the block on GitHub"), "{text}");

    let out = h.app.wf.answer(&card, "merge").await.unwrap();

    assert_eq!(out["merge"]["status"], "waiting", "{out}");
    assert_eq!(out["merge"]["mergeState"], "BLOCKED");
    h.app.wf.reconcile().await.unwrap();
    assert!(mutations(&h).is_empty(), "t3cm never merges over a review: {:?}", mutations(&h));
    assert!(to(&h, TICKET).is_empty());

    h.github.update_pr(REPO, 7, json!({"state": "MERGED", "mergeCommit": {"oid": MOVED}, "mergedAt": at(5)}));
    h.app.wf.reconcile().await.unwrap();

    assert!(to(&h, WORKER).iter().any(|m| m.contains(&handoff(MOVED))), "{:?}", to(&h, WORKER));
    assert!(to(&h, TICKET).is_empty(), "{:?}", to(&h, TICKET));
    assert_eq!(mutations(&h), ["deleteRef"]);
}

#[tokio::test]
async fn a_merge_on_github_with_no_answer_counts_as_merge_alone() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some("rebase")).await;
    open_pr(&h, false);
    ready_card(&h).await;
    h.github.update_pr(
        REPO,
        7,
        json!({"state": "MERGED", "mergeCommit": {"oid": MOVED}, "mergedAt": at(5), "headRef": null}),
    );

    h.app.wf.reconcile().await.unwrap();

    assert!(cards(&h).is_empty(), "{:?}", cards(&h));
    assert!(mutations(&h).is_empty(), "GitHub already deleted the branch: {:?}", mutations(&h));
    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 1, "{woke:?}");
    assert!(woke[0].contains("merged on GitHub at 2222222, outside t3cm"), "{}", woke[0]);
    assert!(woke[0].contains("found branch feat/7 already deleted on GitHub"), "{}", woke[0]);
    assert!(woke[0].contains("The user's answer approved none of them."), "{}", woke[0]);
    assert!(to(&h, WORKER).is_empty());
}

#[tokio::test]
async fn a_merge_that_github_refuses_goes_to_the_ticket() {
    let h = setup(merge_only());
    ticket_and_worker(&h, "1/2", Some("rebase")).await;
    open_pr(&h, false);
    let card = ready_card(&h).await;
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));

    let out = h.app.wf.answer(&card, "merge").await.unwrap();

    assert_eq!(out["merge"]["status"], "refused", "{out}");
    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 1, "{woke:?}");
    assert!(woke[0].contains("The user says \"merge\""), "{}", woke[0]);
    assert!(woke[0].contains("the outcome was refused: head moved"), "{}", woke[0]);
    assert!(woke[0].contains("act on that outcome as \"On the user's word\" item 2 says"), "{}", woke[0]);
    assert!(to(&h, WORKER).is_empty());
}

#[tokio::test]
async fn without_a_merge_method_the_answer_keeps_the_ticket_path() {
    let h = setup(merge_only());
    ticket_and_worker(&h, "1/2", None).await;
    open_pr(&h, false);
    let card = ready_card(&h).await;

    h.app.wf.answer(&card, "merge").await.unwrap();

    assert!(mutations(&h).is_empty());
    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 1);
    assert!(woke[0].contains("The user says \"merge\""), "{}", woke[0]);
}

#[tokio::test]
async fn open_child_rejects_an_unknown_merge_method() {
    let h = setup(merge_only());
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    let err = h
        .wf_err(
            TICKET,
            "openChild",
            json!({"role": "worker", "label": "w", "pr": {"format": "pr", "method": "octopus"}}),
        )
        .await;
    assert!(err.contains("pr.method must be merge, squash or rebase"), "{err}");
}

#[tokio::test]
async fn with_the_reconcile_off_the_answer_keeps_the_ticket_path() {
    let config = Config::from_value(json!({"board": {"reconcileIdleMinutes": 0}})).unwrap();
    let h = prepare(H::build(Vec::new(), config), merge_only());
    ticket_and_worker(&h, "1/2", Some("rebase")).await;
    open_pr(&h, false);
    let card = ready_card(&h).await;

    h.app.wf.answer(&card, "merge").await.unwrap();

    assert!(mutations(&h).is_empty(), "nothing would follow a held approval: {:?}", mutations(&h));
    assert!(to(&h, TICKET)[0].contains("The user says \"merge\""));
}

#[tokio::test]
async fn a_fix_answer_on_a_held_approval_drops_it_so_nothing_merges() {
    let h = setup(merge_only());
    ticket_and_worker(&h, "1/2", Some("rebase")).await;
    open_pr(&h, false);
    let card = ready_card(&h).await;
    let running = json!({"__typename": "CheckRun", "name": "e2e", "status": "IN_PROGRESS", "conclusion": null});
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS"), running]));
    assert_eq!(h.app.wf.answer(&card, "merge").await.unwrap()["merge"]["status"], "waiting");
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS"), check_run("e2e", "FAILURE")]));
    h.app.wf.reconcile().await.unwrap();
    let held = cards(&h);
    assert_eq!(held[0]["options"], json!(["fix"]), "{held:?}");

    h.app.wf.answer(held[0]["id"].as_str().unwrap(), "fix").await.unwrap();
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS"), check_run("e2e", "SUCCESS")]));
    h.app.wf.reconcile().await.unwrap();

    assert!(mutations(&h).is_empty(), "{:?}", mutations(&h));
    assert!(h.app.store.merge_waits().unwrap().is_empty());
    assert!(to(&h, TICKET).iter().any(|m| m.contains("The user says \"fix\"")));
}

#[tokio::test]
async fn a_merge_at_a_newer_heads_open_card_counts_even_with_an_older_approval() {
    let h = setup(merge_only());
    ticket_and_worker(&h, "1/2", Some("rebase")).await;
    open_pr(&h, false);
    let card = ready_card(&h).await;
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "BLOCKED"}));
    h.app.wf.answer(&card, "merge").await.unwrap();
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED, "mergeStateStatus": "CLEAN"}));
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS")]));
    h.wf(WORKER, "pr", json!({"event": "updated", "repo": REPO, "pr": 7, "head": MOVED})).await;
    let r = h.wf(WORKER, "pr", json!({"event": "ready", "repo": REPO, "pr": 7, "head": MOVED})).await;
    assert_eq!(r["status"], "gated", "{r}");
    h.github.update_pr(REPO, 7, json!({"state": "MERGED", "mergeCommit": {"oid": HEAD}, "mergedAt": at(5)}));

    h.app.wf.reconcile().await.unwrap();

    assert!(to(&h, WORKER).iter().any(|m| m.contains(&handoff(HEAD))), "{:?}", to(&h, WORKER));
    assert!(!to(&h, TICKET).iter().any(|m| m.contains("Run your post-merge steps")), "{:?}", to(&h, TICKET));
}

#[tokio::test]
async fn a_check_rerunning_on_a_clean_pr_holds_the_merge() {
    let h = setup(merge_only());
    ticket_and_worker(&h, "1/2", Some("rebase")).await;
    open_pr(&h, false);
    let card = ready_card(&h).await;
    let running = json!({"__typename": "CheckRun", "name": "CodeRabbit", "status": "IN_PROGRESS", "conclusion": null});
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS"), running]));

    let out = h.app.wf.answer(&card, "merge").await.unwrap();

    assert_eq!(out["merge"]["status"], "waiting", "{out}");
    h.app.wf.reconcile().await.unwrap();
    assert!(mutations(&h).is_empty(), "{:?}", mutations(&h));
}

#[tokio::test]
async fn an_ok_head_that_leaves_the_queue_can_be_requeued_in_code() {
    let h = setup(merge_only());
    ticket_and_worker(&h, "1/2", Some("merge")).await;
    open_pr(&h, true);
    let card = ready_card(&h).await;
    h.app.wf.answer(&card, "merge").await.unwrap();
    h.app.wf.reconcile().await.unwrap();
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS")]));
    h.app.wf.reconcile().await.unwrap();
    h.app.wf.answer(cards(&h)[0]["id"].as_str().unwrap(), "ok").await.unwrap();
    h.clock.set(&at(3));
    h.github.drop_from_queue(REPO, 7, "manual", &at(2));
    h.app.wf.reconcile().await.unwrap();
    let dropped = cards(&h);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert!(dropped[0]["approval"].is_string(), "{dropped:?}");

    let out = h.app.wf.answer(dropped[0]["id"].as_str().unwrap(), "requeue").await.unwrap();

    assert_eq!(out["merge"]["status"], "queued", "{out}");
    assert_eq!(out["merge"]["head"], MOVED);
    assert!(to(&h, TICKET).is_empty(), "{:?}", to(&h, TICKET));
}

#[tokio::test]
async fn a_branch_pushed_after_the_merge_is_left_alone() {
    let h = setup(merge_only());
    ticket_and_worker(&h, "1/2", Some("rebase")).await;
    open_pr(&h, false);
    ready_card(&h).await;
    h.github.update_pr(
        REPO,
        7,
        json!({"state": "MERGED", "mergeCommit": {"oid": MOVED}, "mergedAt": at(5),
               "headRef": {"id": "REF_acme/widgets_7", "target": {"oid": MOVED}}}),
    );

    h.app.wf.reconcile().await.unwrap();

    assert!(mutations(&h).is_empty(), "{:?}", mutations(&h));
    let handled = digest(&h).await;
    assert!(
        handled.iter().any(|l| l.contains("left branch feat/7, as its tip 2222222 is not the merged")),
        "{handled:?}"
    );
}

#[tokio::test]
async fn a_method_alone_is_a_valid_pr_brief() {
    let h = setup(merge_only());
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(TICKET, "openChild", json!({"role": "worker", "label": "w", "pr": {"method": "rebase"}})).await;
}

#[tokio::test]
async fn stages_approved_for_one_head_do_not_carry_to_an_accepted_queued_head() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some("merge")).await;
    open_pr(&h, true);
    let card = ready_card(&h).await;
    h.app.wf.answer(&card, "merge and deploy").await.unwrap();
    h.app.wf.reconcile().await.unwrap();
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS")]));
    h.app.wf.reconcile().await.unwrap();
    h.app.wf.answer(cards(&h)[0]["id"].as_str().unwrap(), "ok").await.unwrap();
    h.github.update_pr(
        REPO,
        7,
        json!({"state": "MERGED", "isInMergeQueue": false, "mergeCommit": {"oid": HEAD}, "mergedAt": at(5)}),
    );

    h.app.wf.reconcile().await.unwrap();

    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 1, "{woke:?}");
    assert!(woke[0].contains("The user's answer approved none of them."), "{}", woke[0]);
}

async fn answered_board(h: &H) -> String {
    ticket_and_worker(h, "1/2", Some("squash")).await;
    open_pr(h, false);
    ready_card(h).await
}

#[tokio::test]
async fn a_crash_after_an_answered_merge_lands_still_tells_the_ticket_and_drops_the_record() {
    use futures_util::FutureExt;
    let clean = setup(merge_and_deploy());
    let card = answered_board(&clean).await;
    let before = clean.app.store.writes();
    clean.app.wf.answer(&card, "merge and deploy").await.unwrap();
    let writes = clean.app.store.writes() - before;
    let mut landed = 0;

    for k in 1..=writes {
        let mut h = setup(merge_and_deploy());
        let card = answered_board(&h).await;
        h.app.store.set_fault(Some(h.app.store.writes() + k));
        let answer = h.app.wf.answer(&card, "merge and deploy");
        assert!(std::panic::AssertUnwindSafe(answer).catch_unwind().await.is_err(), "write {k} did not crash");
        h.reopen();
        h.app.wf.reconcile().await.unwrap();
        h.app.wf.tick().await.unwrap();
        if h.github.pr(REPO, 7)["state"] != "MERGED" {
            continue;
        }
        landed += 1;
        let woke = to(&h, TICKET);
        assert!(
            woke.iter().any(|m| m.contains("#41 PR 1/2 is merged")),
            "a crash at answer write {k} of {writes} never told the ticket: {woke:?}"
        );
        let told: std::collections::BTreeSet<&str> =
            woke.iter().filter(|m| m.contains("#41 PR 1/2 is merged")).filter_map(|m| m.lines().next()).collect();
        assert_eq!(told.len(), 1, "write {k} told the ticket in more than one message: {woke:?}");
        assert_eq!(h.app.store.meta_get("merge_answer:acme/widgets#7").unwrap(), None, "write {k} kept the record");
        assert!(cards(&h).is_empty(), "write {k}: {:?}", cards(&h));
    }
    assert!(landed > 0, "no crash point landed after the merge");
}
