use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::BOARD;
use t3_codemode::config::Config;
use t3_codemode::fake::{check_run, github_pr, project, pull_request, thread};

const REPO: &str = "acme/widgets";
const HEAD: &str = "1111111111111111111111111111111111111111";
const MOVED: &str = "2222222222222222222222222222222222222222";
const PUBLISHING: &str = "Publishing: pr skill; passes: none configured";
const GOOD: &str = "## Summary\n\n```text\nwf.pr\n  opened\n```\n\n## Evidence\n\n- **Before:** none\n  **After:** green\n\n\
## Merge Danger\n\n**Door:** two-way\n\n**Blast Radius:** board\n\n## Compatibility\n\nNone\n\nRefs #41\n";

fn pending_check(name: &str) -> Value {
    json!({"__typename": "CheckRun", "name": name, "status": "IN_PROGRESS", "conclusion": null})
}

fn setup(config: Value) -> H {
    let h = H::new();
    let home = h.dir.path().join("home");
    let repo = h.dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    for skill in ["deploy", "verify", "publish"] {
        let d = home.join(".claude/skills").join(skill);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), format!("---\nname: {skill}\ndescription: test\n---\n\nBody.\n")).unwrap();
    }
    let p = home.join(".config/t3-workflow/config.json");
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, config.to_string()).unwrap();
    h.fake.set_projects(vec![project("proj:widgets", &repo.to_string_lossy())]);
    let mut th = thread(WORKER, "idle", T0);
    th["projectId"] = json!("proj:widgets");
    h.fake.put(th);
    h
}

fn merge_and_deploy() -> Value {
    json!({"delivery": {"steps": [
        {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
        {"id": "deploy", "skill": "deploy", "gate": "chain", "scope": "pr"},
        {"id": "publish", "skill": "publish", "gate": "chain", "scope": "ticket"}
    ]}})
}

async fn ticket_and_worker(h: &H, part: &str, pr: Option<Value>) {
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    let mut args =
        json!({"role": "worker", "label": format!("#41 PR {part}"), "issue": "acme/widgets#41", "part": part});
    if let Some(pr) = pr {
        args["pr"] = pr;
    }
    h.wf(TICKET, "openChild", args).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": format!("#41 PR {part}"), "role": "worker"})).await;
}

fn pr_format() -> Value {
    json!({"format": "pr", "sections": {"Compatibility": []}})
}

fn open_pr(h: &H, head: &str, body: &str, files: &[&str]) {
    let mut pr = github_pr(head, "CLEAN", false);
    pr["body"] = json!(body);
    let nodes: Vec<Value> = files.iter().map(|f| json!({"path": f})).collect();
    pr["files"] = json!({"pageInfo": {"hasNextPage": false}, "nodes": nodes});
    h.github.set_pr(REPO, 7, pr);
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS")]));
}

async fn pr_event(h: &H, args: Value) -> Value {
    h.wf(WORKER, "pr", args).await
}

fn opened(head: &str) -> Value {
    json!({"event": "opened", "repo": REPO, "pr": 7, "head": head, "publishing": PUBLISHING, "checks": "pending"})
}

fn ready(head: &str) -> Value {
    json!({"event": "ready", "repo": REPO, "pr": 7, "head": head})
}

fn to(h: &H, thread: &str) -> Vec<String> {
    h.sent().into_iter().filter(|s| s["threadId"] == thread).map(|s| s["message"].as_str().unwrap().into()).collect()
}

fn cards(h: &H) -> Vec<Value> {
    h.app.wf.cards().unwrap().into_iter().map(|c| serde_json::to_value(c).unwrap()).collect()
}

async fn card_body(h: &H, card: &Value) -> String {
    let shown = h.wf(TICKET, "show", json!({"handle": card["handle"]})).await;
    shown["body"].as_str().unwrap().to_string()
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

#[tokio::test]
async fn a_well_formed_opened_pr_is_checked_without_a_ticket_turn() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);

    let r = pr_event(&h, opened(HEAD)).await;

    assert_eq!(r["status"], "open", "{r}");
    assert!(to(&h, TICKET).is_empty(), "opened must not wake the ticket");
    let feed = h.app.wf.feed(20).unwrap();
    assert!(feed.iter().any(|m| m["head"].as_str().unwrap().starts_with("PR 1/2 opened: #7")), "{feed:?}");
    let handled = digest(&h).await;
    assert_eq!(handled.len(), 1, "{handled:?}");
    assert!(handled[0].contains("#41 PR 1/2 opened https://github.com/acme/widgets/pull/7 at head 1111111"));
    assert!(handled[0].contains("checked its description and Publishing line"), "{}", handled[0]);
}

#[tokio::test]
async fn an_opened_pr_missing_sections_gets_the_rewrite_request() {
    let h = setup(json!({"passes": ["deploy"]}));
    ticket_and_worker(
        &h,
        "1/1",
        Some(json!({"format": "pr", "sections": {"Compatibility": []}, "passes": ["verify"]})),
    )
    .await;
    let body = "## Summary\n\nno fence here\n\n## Evidence\n\n**Before:** red\n";
    open_pr(&h, HEAD, body, &["src/lib.rs"]);

    let r = pr_event(&h, opened(HEAD)).await;

    assert_eq!(r["status"], "rewrite", "{r}");
    assert_eq!(
        r["missing"],
        json!([
            "the deploy pass on the Publishing line",
            "the verify pass on the Publishing line",
            "the Compatibility heading",
            "\"After:\" under Evidence",
            "the Merge Danger heading",
            "a fenced block under Summary",
            "a \"Refs #41\" line"
        ])
    );
    let message = r["message"].as_str().unwrap();
    assert!(
        message.starts_with(
            "Rewrite #7's description through the Publishing section of the t3-worker procedure: the deploy pass on"
        ),
        "{message}"
    );
    assert!(to(&h, TICKET).is_empty());
    let handled = digest(&h).await;
    assert!(
        handled[0].contains("(m1); t3cm sent the worker the rewrite request, as it is missing the deploy pass"),
        "{handled:?}"
    );

    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    let publishing = "Publishing: pr skill; passes: deploy ok, verify ok";
    let again =
        pr_event(&h, json!({"event": "opened", "repo": REPO, "pr": 7, "head": HEAD, "publishing": publishing})).await;
    assert_eq!(again["status"], "open", "{again}");
}

#[tokio::test]
async fn opened_without_a_pr_format_still_wakes_the_ticket() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/1", None).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);

    let r = pr_event(&h, opened(HEAD)).await;

    assert_eq!(r["status"], "ticket");
    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 1);
    assert!(woke[0].contains("PR 1/1 opened: #7 https://github.com/acme/widgets/pull/7, head 1111111"), "{}", woke[0]);
    assert!(woke[0].contains("Publishing: pr skill"), "{}", woke[0]);
    assert!(woke[0].contains("Checks: pending"), "{}", woke[0]);
    assert!(woke[0].contains("t3cm left this to the ticket: the ticket's wf.openChild gave no PR format"));
}

#[tokio::test]
async fn ready_opens_the_merge_card_with_the_delivery_chain() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);

    let r = pr_event(&h, ready(&HEAD[..7])).await;

    assert_eq!(r["status"], "gated", "{r}");
    assert_eq!(r["options"], json!(["merge and deploy", "merge"]));
    assert!(to(&h, TICKET).is_empty(), "ready must not wake the ticket");
    let open = cards(&h);
    assert_eq!(open.len(), 1);
    let card = &open[0];
    assert_eq!(card["kind"], "gate");
    assert_eq!(card["ticket"], TICKET);
    assert_eq!(card["label"], "widgets#41");
    assert!(card.get("worker").is_none(), "the merge card is the ticket's own gate");
    assert_eq!(card["options"], json!(["merge and deploy", "merge"]));
    let text = card["text"].as_str().unwrap();
    assert!(
        text.starts_with(
            "merge #7 at head 1111111: reply \"merge and deploy\" to merge and then run deploy, \
             or \"merge\" to merge only, or review it first, here: https://github.com/acme/widgets/pull/7"
        ),
        "{text}"
    );
    let body = card_body(&h, card).await;
    assert!(body.starts_with("**ACTION NEEDED:** merge #7 at head 1111111: reply"), "{body}");
    assert!(body.contains(&format!("Every check passed at head {HEAD}.")), "{body}");
    assert!(body.contains("Later stages open their own gate: publish."), "{body}");
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("merge"));
    assert_eq!(gate_of(&h, WORKER).as_deref(), Some("merge"));
    h.tick(200).await;
    assert!(h.sent().is_empty(), "neither thread is nudged while the merge card waits: {:?}", h.sent());

    let id = card["id"].as_str().unwrap();
    h.app.wf.answer(id, "merge and deploy").await.unwrap();
    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 1, "the user's answer reaches the ticket");
    assert!(woke[0].contains("The user says \"merge and deploy\""), "{}", woke[0]);
    assert!(woke[0].contains("- handled for you"), "{}", woke[0]);
    assert!(woke[0].contains(&format!("opened merge card {id} with \"merge and deploy\", \"merge\"")), "{}", woke[0]);
}

#[tokio::test]
async fn the_last_pr_chains_into_ticket_stages_and_stops_at_a_manual_one() {
    let h = setup(json!({"delivery": {"steps": [
        {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
        {"id": "deploy", "skill": "deploy", "gate": "chain", "scope": "pr"},
        {"id": "publish", "skill": "publish", "gate": "chain", "scope": "ticket"},
        {"id": "signoff", "kind": "manual", "prompt": "sign it off", "gate": "chain", "scope": "ticket"},
        {"id": "verify", "skill": "verify", "gate": "chain", "scope": "ticket"}
    ]}}));
    ticket_and_worker(&h, "2/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);

    let r = pr_event(&h, ready(HEAD)).await;

    assert_eq!(r["options"], json!(["merge, deploy and publish", "merge and deploy", "merge"]), "{r}");
    let card = cards(&h).remove(0);
    let text = card["text"].as_str().unwrap().to_string();
    assert!(text.contains("\"merge, deploy and publish\" to merge and then run deploy and publish"), "{text}");
    let body = card_body(&h, &card).await;
    assert!(body.contains("Later stages open their own gate: signoff, verify."), "{body}");
}

#[tokio::test]
async fn ready_drops_stages_whose_paths_miss_and_keeps_to_the_option_limits() {
    let long = "a-stage-with-a-name-long-enough-to-pass-the-limit";
    let mut steps = vec![
        json!({"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"}),
        json!({"id": "docs", "skill": "publish", "gate": "chain", "scope": "pr", "paths": ["docs/**"]}),
    ];
    for i in 1..=8 {
        steps.push(json!({"id": format!("s{i}"), "skill": "deploy", "gate": "chain", "scope": "pr"}));
    }
    steps.push(json!({"id": long, "skill": "deploy", "gate": "chain", "scope": "pr"}));
    let h = setup(json!({"delivery": {"steps": steps}}));
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);

    let r = pr_event(&h, ready(HEAD)).await;

    let options: Vec<String> = serde_json::from_value(r["options"].clone()).unwrap();
    assert_eq!(options.len(), 8, "{options:?}");
    assert_eq!(options.last().map(String::as_str), Some("merge"));
    assert_eq!(options[0], "merge, s1, s2, s3, s4, s5, s6 and s7");
    assert!(options.iter().all(|o| o.chars().count() <= 60 && !o.contains("docs")), "{options:?}");
    let card = cards(&h).remove(0);
    let text = card["text"].as_str().unwrap().to_string();
    assert!(text.contains("to merge and then run the stages each answer names, or review it first"), "{text}");
    let body = card_body(&h, &card).await;
    assert!(body.contains("Skipped for this PR: docs (no changed file matches its paths)."), "{body}");
}

#[tokio::test]
async fn a_blocked_merge_state_names_the_blocker_on_the_card() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "BLOCKED", "reviewDecision": "CHANGES_REQUESTED"}));

    let r = pr_event(&h, ready(HEAD)).await;

    assert_eq!(r["status"], "gated", "{r}");
    assert_eq!(r["options"], json!(["merge and deploy", "merge"]));
    let text = cards(&h)[0]["text"].as_str().unwrap().to_string();
    assert!(
        text.starts_with(
            "merge #7 at head 1111111 (GitHub blocks it: merge state BLOCKED, review decision \
             CHANGES_REQUESTED; an admin can merge it over the block on GitHub, and the board picks that up): reply"
        ),
        "{text}"
    );
    assert!(to(&h, TICKET).is_empty());
}

#[tokio::test]
async fn a_failing_check_goes_back_to_the_worker() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([check_run("test", "FAILURE")]));

    let r = pr_event(&h, ready(HEAD)).await;

    assert_eq!(r["status"], "failing");
    assert_eq!(r["checks"], "test (FAILURE)");
    assert!(cards(&h).is_empty());
    assert!(to(&h, TICKET).is_empty());
    let handled = digest(&h).await;
    assert!(handled[0].contains("a check failed: test (FAILURE); t3cm sent it back to the worker"), "{handled:?}");
}

#[tokio::test]
async fn a_pending_check_waits_and_reconcile_opens_the_card_when_it_passes() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));

    let r = pr_event(&h, ready(HEAD)).await;
    assert_eq!(r["status"], "waiting", "{r}");
    assert_eq!(r["pending"], "checks still running: test");
    assert!(cards(&h).is_empty());

    h.app.wf.reconcile().await.unwrap();
    assert!(cards(&h).is_empty(), "still pending: no card yet");

    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS")]));
    h.app.wf.reconcile().await.unwrap();
    let open = cards(&h);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0]["options"], json!(["merge and deploy", "merge"]));
    assert!(to(&h, TICKET).is_empty(), "the card opens with no ticket turn");
    assert!(to(&h, WORKER).is_empty());
}

#[tokio::test]
async fn a_pending_check_that_fails_later_comes_back_to_the_worker() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));
    pr_event(&h, ready(HEAD)).await;

    h.github.set_checks(REPO, 7, json!([check_run("test", "FAILURE")]));
    h.app.wf.reconcile().await.unwrap();

    assert!(cards(&h).is_empty());
    let back = to(&h, WORKER);
    assert_eq!(back.len(), 1, "{back:?}");
    assert!(back[0].starts_with("[t3cm reply from=t3cm msg="), "{}", back[0]);
    assert!(
        back[0].contains("A check failed on https://github.com/acme/widgets/pull/7 at head 1111111: test (FAILURE)")
    );
    assert!(to(&h, TICKET).is_empty());
    h.app.wf.reconcile().await.unwrap();
    assert_eq!(to(&h, WORKER).len(), 1, "one reply per failure");
}

#[tokio::test]
async fn updated_drops_the_merge_card_for_the_old_head_without_a_ticket_turn() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    let card = pr_event(&h, ready(HEAD)).await["card"].as_str().unwrap().to_string();
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));

    let r =
        pr_event(&h, json!({"event": "updated", "repo": REPO, "pr": 7, "head": MOVED, "body": "fixed a review nit"}))
            .await;

    assert_eq!(r["status"], "recorded");
    assert_eq!(r["dropped"], json!([card]));
    assert!(cards(&h).is_empty());
    assert_eq!(gate_of(&h, TICKET), None);
    assert_eq!(gate_of(&h, WORKER), None);
    assert!(to(&h, TICKET).is_empty(), "updated must not wake the ticket");
    let handled = digest(&h).await;
    let moved = handled.iter().find(|l| l.contains("to head 2222222")).expect("an updated line");
    assert!(moved.contains(": fixed a review nit"), "{moved}");
    assert!(moved.contains(&format!("t3cm dropped merge card {card} for the old head")), "{moved}");
}

#[tokio::test]
async fn ready_at_a_stale_head_is_refused() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, MOVED, GOOD, &["src/lib.rs"]);

    let r = pr_event(&h, ready(HEAD)).await;

    assert_eq!(r["status"], "refused");
    assert!(r["reason"].as_str().unwrap().contains("the head moved"), "{r}");
    assert!(cards(&h).is_empty());
}

#[tokio::test]
async fn a_ticket_under_an_overseer_gets_ready_as_its_merge_gate() {
    let h = setup(merge_and_deploy());
    h.wf(BOSS, "init", json!({"role": "overseer"})).await;
    h.wf(TICKET, "join", json!({"parent": BOSS, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(TICKET, "openChild", json!({"role": "worker", "label": "#41 PR 1/1", "part": "1/1", "pr": pr_format()})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);

    let r = pr_event(&h, ready(HEAD)).await;

    assert_eq!(r["status"], "ticket");
    assert_eq!(gate_of(&h, WORKER).as_deref(), Some("merge"));
    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 1);
    assert!(woke[0].starts_with("[t3cm gate from=\"#41 PR 1/1\" msg="), "{}", woke[0]);
    assert!(woke[0].contains("gate=merge]"), "{}", woke[0]);
    assert!(
        woke[0].contains("PR 1/1 ready to merge: #7, head 1111111111111111111111111111111111111111, checks passed")
    );
    assert!(woke[0].contains("the ticket reports to an overseer thread"), "{}", woke[0]);
}

#[tokio::test]
async fn relay_off_and_merge_only_keep_the_ticket_path() {
    let mut config = Config::default();
    config.roles.entry("ticket".into()).or_default().children.entry("worker".into()).or_default().relay = Some(false);
    let h = H::build(Vec::new(), config);
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    let r = pr_event(&h, ready(HEAD)).await;
    assert_eq!(r["status"], "ticket");
    assert!(r["why"].as_str().unwrap().contains("relay is off"), "{r}");

    let h = setup(merge_and_deploy());
    let fix = json!({"format": "pr", "mergeOnly": true});
    ticket_and_worker(&h, "1/1", Some(fix)).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    let r = pr_event(&h, ready(HEAD)).await;
    assert_eq!(r["status"], "ticket");
    assert!(r["why"].as_str().unwrap().contains("targeted fix PR"), "{r}");
    assert!(cards(&h).is_empty());
    assert_eq!(to(&h, TICKET).len(), 1);
}

#[tokio::test]
async fn a_delivery_slot_with_no_builtin_merge_keeps_the_ticket_path() {
    let h = setup(json!({"delivery": {"steps": [{"id": "ship", "skill": "deploy", "gate": "always", "scope": "pr"}]}}));
    ticket_and_worker(&h, "1/1", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);

    let r = pr_event(&h, ready(HEAD)).await;

    assert_eq!(r["status"], "ticket", "{r}");
    assert!(r["why"].as_str().unwrap().contains("no builtin merge stage"), "{r}");
    assert!(cards(&h).is_empty());
}

#[tokio::test]
async fn a_second_pr_from_one_worker_goes_to_the_ticket() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    pr_event(&h, opened(HEAD)).await;

    let r =
        pr_event(&h, json!({"event": "opened", "repo": REPO, "pr": 9, "head": MOVED, "publishing": PUBLISHING})).await;

    assert_eq!(r["status"], "ticket");
    assert!(r["why"].as_str().unwrap().contains("follow-up PR"), "{r}");
}

#[tokio::test]
async fn bad_pr_events_are_refused() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/1", Some(pr_format())).await;
    let err = h.wf_err(WORKER, "pr", json!({"event": "merged", "repo": REPO, "pr": 7, "head": HEAD})).await;
    assert!(err.contains("event must be opened, updated or ready"), "{err}");
    let err = h.wf_err(WORKER, "pr", json!({"event": "opened", "repo": REPO, "pr": 7, "head": HEAD})).await;
    assert!(err.contains("opened needs publishing"), "{err}");
    let err = h.wf_err(WORKER, "pr", json!({"event": "ready", "repo": REPO, "pr": 7, "head": "xyz"})).await;
    assert!(err.contains("head must be the head commit SHA"), "{err}");
    let err = h.wf_err(TICKET, "openChild", json!({"role": "worker", "label": "x", "pr": {"format": " "}})).await;
    assert!(err.contains("pr.format must name"), "{err}");
}

#[tokio::test]
async fn a_push_seen_by_reconcile_drops_the_unanswered_card_without_a_generic_one() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    pr_event(&h, ready(HEAD)).await;
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));

    h.app.wf.reconcile().await.unwrap();

    assert!(cards(&h).is_empty(), "no merge card for an unverified head: {:?}", cards(&h));
    assert_eq!(gate_of(&h, TICKET), None);
    assert!(to(&h, TICKET).is_empty());
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS")]));
    pr_event(&h, json!({"event": "updated", "repo": REPO, "pr": 7, "head": MOVED})).await;
    let r = pr_event(&h, ready(MOVED)).await;
    assert_eq!(r["status"], "gated", "{r}");
    assert_eq!(r["options"], json!(["merge and deploy", "merge"]));
}

#[tokio::test]
async fn a_waiting_pr_that_falls_back_to_the_ticket_tells_it_once() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));
    pr_event(&h, ready(HEAD)).await;
    h.github.set_checks(REPO, 7, json!([]));

    h.app.wf.reconcile().await.unwrap();
    h.app.wf.reconcile().await.unwrap();

    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 1, "{woke:?}");
    assert!(woke[0].contains("cannot verify the checks"), "{}", woke[0]);
}

#[tokio::test]
async fn refs_must_name_the_issue_outside_a_code_fence() {
    let h = setup(json!({}));
    ticket_and_worker(&h, "1/1", Some(pr_format())).await;
    let wrong = GOOD.replace("Refs #41\n", "Refs other/repo#41\n\n```text\nRefs #41\n```\n");
    open_pr(&h, HEAD, &wrong, &["src/lib.rs"]);
    let r = pr_event(&h, opened(HEAD)).await;
    assert_eq!(r["missing"], json!(["a \"Refs #41\" line"]), "{r}");

    open_pr(&h, HEAD, &GOOD.replace("Refs #41", "Refs https://github.com/acme/widgets/issues/41."), &["src/lib.rs"]);
    assert_eq!(pr_event(&h, opened(HEAD)).await["status"], "open");
}

#[tokio::test]
async fn a_stale_ready_leaves_the_newer_waiting_head_alone() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, MOVED, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));
    assert_eq!(pr_event(&h, ready(MOVED)).await["status"], "waiting");

    let stale = pr_event(&h, ready(HEAD)).await;
    assert_eq!(stale["status"], "refused", "{stale}");

    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS")]));
    h.app.wf.reconcile().await.unwrap();
    let open = cards(&h);
    assert_eq!(open.len(), 1, "the newer head still gets its card: {open:?}");
    assert!(open[0]["text"].as_str().unwrap().contains("at head 2222222"), "{}", open[0]["text"]);
}

#[tokio::test]
async fn a_refs_line_inside_a_longer_fence_does_not_count() {
    let h = setup(json!({}));
    ticket_and_worker(&h, "1/1", Some(pr_format())).await;
    let body = GOOD.replace("Refs #41\n", "````markdown\n```\nRefs #41\n````\n");
    open_pr(&h, HEAD, &body, &["src/lib.rs"]);
    let r = pr_event(&h, opened(HEAD)).await;
    assert_eq!(r["missing"], json!(["a \"Refs #41\" line"]), "{r}");
}

#[tokio::test]
async fn an_update_during_a_ready_check_wins_over_the_older_head() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.fake.set_delay(std::time::Duration::from_millis(300));
    let update = async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        pr_event(&h, json!({"event": "updated", "repo": REPO, "pr": 7, "head": MOVED})).await
    };

    let (r, u) = tokio::join!(pr_event(&h, ready(HEAD)), update);

    assert_eq!(u["status"], "recorded", "{u}");
    assert_eq!(r["status"], "superseded", "{r}");
    assert!(cards(&h).is_empty(), "no card for the older head: {:?}", cards(&h));
    let pr = h.app.store.link_get(WORKER).unwrap().unwrap().pr.unwrap();
    assert_eq!(pr.head, MOVED);
}

#[tokio::test]
async fn a_targeted_fix_with_no_pr_format_still_keeps_merge_only_delivery() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/1", Some(json!({"mergeOnly": true}))).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);

    let opened = pr_event(&h, opened(HEAD)).await;
    assert_eq!(opened["status"], "ticket", "{opened}");
    let r = pr_event(&h, ready(HEAD)).await;

    assert_eq!(r["status"], "ticket", "{r}");
    assert!(r["why"].as_str().unwrap().contains("targeted fix PR"), "{r}");
    assert!(cards(&h).is_empty());
}

#[tokio::test]
async fn an_update_in_a_repo_with_no_ci_goes_to_the_ticket() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    let mut args = opened(HEAD);
    args["checks"] = json!("none");
    assert_eq!(pr_event(&h, args).await["status"], "ticket");

    let r = pr_event(&h, json!({"event": "updated", "repo": REPO, "pr": 7, "head": MOVED})).await;

    assert_eq!(r["status"], "ticket", "{r}");
    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 2, "{woke:?}");
    assert!(woke[1].contains("PR 1/2 updated: #7, head 2222222"), "{}", woke[1]);
    assert!(woke[1].contains("runs the brief's checks on the new head"), "{}", woke[1]);
}

#[tokio::test]
async fn a_waiting_pr_that_github_no_longer_has_goes_to_the_ticket_once() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));
    pr_event(&h, ready(HEAD)).await;
    h.github.remove_pr(REPO, 7);

    let _ = h.app.wf.reconcile().await;
    let _ = h.app.wf.reconcile().await;

    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 1, "{woke:?}");
    assert!(woke[0].contains("could not read acme/widgets#7 on GitHub"), "{}", woke[0]);
    assert!(!h.app.store.link_get(WORKER).unwrap().unwrap().pr.unwrap().waiting);
}

#[tokio::test]
async fn a_failed_check_reaches_the_worker_while_the_merge_state_is_unknown() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));
    pr_event(&h, ready(HEAD)).await;
    h.github.set_checks(REPO, 7, json!([check_run("test", "FAILURE")]));
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "UNKNOWN"}));

    h.app.wf.reconcile().await.unwrap();

    let back = to(&h, WORKER);
    assert_eq!(back.len(), 1, "{back:?}");
    assert!(back[0].contains("A check failed on https://github.com/acme/widgets/pull/7"), "{}", back[0]);
}

#[tokio::test]
async fn the_last_pr_counts_files_from_an_earlier_workers_follow_up_pr() {
    let h = setup(json!({"delivery": {"steps": [
        {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
        {"id": "publish", "skill": "publish", "gate": "chain", "scope": "ticket", "paths": ["docs/**"]}
    ]}}));
    let earlier = "mcp:earlier";
    let mut th = thread(earlier, "idle", T0);
    th["projectId"] = json!("proj:widgets");
    h.fake.put(th);
    h.wf(TICKET, "join", json!({"parent": BOARD, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(
        TICKET,
        "openChild",
        json!({"role": "worker", "label": "#41 PR 1/2", "issue": "acme/widgets#41", "part": "1/2", "pr": pr_format()}),
    )
    .await;
    h.wf(earlier, "join", json!({"parent": TICKET, "label": "#41 PR 1/2", "role": "worker"})).await;
    let mut first = github_pr(HEAD, "MERGED", false);
    first["files"] = json!({"pageInfo": {"hasNextPage": false}, "nodes": [{"path": "src/a.rs"}]});
    h.github.set_pr(REPO, 5, first);
    let mut fix = github_pr(MOVED, "MERGED", false);
    fix["files"] = json!({"pageInfo": {"hasNextPage": false}, "nodes": [{"path": "docs/runbook.md"}]});
    h.github.set_pr(REPO, 6, fix);
    h.wf(earlier, "pr", json!({"event": "updated", "repo": REPO, "pr": 5, "head": HEAD})).await;
    h.wf(earlier, "pr", json!({"event": "updated", "repo": REPO, "pr": 6, "head": MOVED})).await;
    h.wf(earlier, "send", json!({"kind": "handoff", "body": "done"})).await;
    h.wf(
        TICKET,
        "openChild",
        json!({"role": "worker", "label": "#41 PR 2/2", "issue": "acme/widgets#41", "part": "2/2", "pr": pr_format()}),
    )
    .await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 2/2", "role": "worker"})).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);

    let r = pr_event(&h, ready(HEAD)).await;

    assert_eq!(r["options"], json!(["merge and publish", "merge"]), "{r}");
}

#[tokio::test]
async fn an_adopting_ticket_takes_over_the_merge_card_its_worker_got() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    let card = pr_event(&h, ready(HEAD)).await["card"].as_str().unwrap().to_string();
    let heir = "mcp:heir";
    h.fake.put(thread(heir, "idle", T0));
    h.wf(heir, "join", json!({"parent": BOARD, "label": "widgets#41 again", "role": "ticket"})).await;

    h.wf(heir, "adopt", json!({"from": TICKET})).await;

    assert_eq!(cards(&h)[0]["ticket"], heir);
    assert_eq!(gate_of(&h, heir).as_deref(), Some("merge"));
    h.app.wf.answer(&card, "merge").await.unwrap();
    assert_eq!(to(&h, heir).len(), 1, "the answer reaches the adopting ticket");
    assert!(to(&h, TICKET).is_empty());
}

#[tokio::test]
async fn a_moved_head_drops_the_ready_card_while_the_ticket_holds_another_gate() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    h.wf(TICKET, "gate", json!({"open": "delivery:verify"})).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    assert_eq!(pr_event(&h, ready(HEAD)).await["status"], "gated");
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));

    h.app.wf.reconcile().await.unwrap();

    assert!(cards(&h).is_empty(), "{:?}", cards(&h));
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("delivery:verify"));
}

#[tokio::test]
async fn a_failed_batch_read_leaves_a_wait_alone_even_with_an_older_head_cached() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));
    pr_event(&h, ready(HEAD)).await;
    h.app.wf.reconcile().await.unwrap();
    open_pr(&h, MOVED, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));
    pr_event(&h, json!({"event": "updated", "repo": REPO, "pr": 7, "head": MOVED})).await;
    assert_eq!(pr_event(&h, ready(MOVED)).await["status"], "waiting");
    h.github.fail_read(REPO, 7, "something went wrong");
    h.github.fail_operation("read", "something went wrong");

    let _ = h.app.wf.reconcile().await;

    assert!(to(&h, TICKET).is_empty(), "{:?}", to(&h, TICKET));
    let pr = h.app.store.link_get(WORKER).unwrap().unwrap().pr.unwrap();
    assert!(pr.waiting && pr.head == MOVED, "{pr:?}");
}

#[tokio::test]
async fn ready_in_a_repo_with_no_ci_goes_to_the_ticket_even_with_a_green_status() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    let mut args = opened(HEAD);
    args["checks"] = json!("none");
    pr_event(&h, args).await;

    let r = pr_event(&h, ready(HEAD)).await;

    assert_eq!(r["status"], "ticket", "{r}");
    assert!(r["why"].as_str().unwrap().contains("runs no CI"), "{r}");
    assert!(cards(&h).is_empty());
}

#[tokio::test]
async fn a_repeated_update_for_the_waiting_head_keeps_the_wait() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));
    assert_eq!(pr_event(&h, ready(HEAD)).await["status"], "waiting");

    let u = pr_event(&h, json!({"event": "updated", "repo": REPO, "pr": 7, "head": &HEAD[..7]})).await;
    assert_eq!(u["status"], "recorded", "{u}");
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS")]));
    h.app.wf.reconcile().await.unwrap();

    assert_eq!(cards(&h).len(), 1, "{:?}", cards(&h));
    assert_eq!(h.app.store.link_get(WORKER).unwrap().unwrap().pr.unwrap().head, HEAD);
}

#[tokio::test]
async fn a_worker_link_closed_during_a_ready_check_gets_no_card() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.fake.set_delay(std::time::Duration::from_millis(300));
    let close = async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        h.wf(TICKET, "close", json!({"child": WORKER})).await
    };

    let (r, _) = tokio::join!(pr_event(&h, ready(HEAD)), close);

    assert_eq!(r["status"], "superseded", "{r}");
    assert!(cards(&h).is_empty(), "{:?}", cards(&h));
    let wl = h.app.store.link_get(WORKER).unwrap().unwrap();
    assert!(wl.closed && wl.gate.is_none(), "{wl:?}");
}

#[tokio::test]
async fn a_push_from_outside_the_worker_releases_its_merge_gate() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    assert_eq!(pr_event(&h, ready(HEAD)).await["status"], "gated");
    assert_eq!(gate_of(&h, WORKER).as_deref(), Some("merge"));
    h.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));

    h.app.wf.reconcile().await.unwrap();

    assert!(cards(&h).is_empty(), "{:?}", cards(&h));
    assert_eq!(gate_of(&h, WORKER), None);
    let back = to(&h, WORKER);
    assert_eq!(back.len(), 1, "{back:?}");
    assert!(back[0].contains("moved to 2222222 outside this thread"), "{}", back[0]);
}

#[tokio::test]
async fn a_merge_seen_while_a_ticket_adopts_the_worker_reaches_the_adopting_ticket() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    assert_eq!(pr_event(&h, ready(HEAD)).await["status"], "gated");
    let heir = "mcp:heir";
    h.fake.put(thread(heir, "idle", T0));
    h.wf(heir, "join", json!({"parent": BOARD, "label": "widgets#41 again", "role": "ticket"})).await;
    h.github.update_pr(REPO, 7, json!({"state": "MERGED", "mergeCommit": {"oid": MOVED}}));
    h.github.set_delay(std::time::Duration::from_millis(300));
    let adopt = async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        h.wf(heir, "adopt", json!({"from": TICKET})).await
    };

    let (r, _) = tokio::join!(h.app.wf.reconcile(), adopt);
    r.unwrap();

    assert!(cards(&h).is_empty(), "{:?}", cards(&h));
    let woke = to(&h, heir);
    assert!(woke.iter().any(|m| m.contains("merged on GitHub")), "{woke:?}");
    assert!(
        to(&h, TICKET).is_empty(),
        "the retiring ticket must not get post-merge instructions: {:?}",
        to(&h, TICKET)
    );
}

#[tokio::test]
async fn a_ready_card_preserves_an_unrelated_ticket_gate_card() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    let other = h
        .wf(TICKET, "send", json!({"kind": "gate", "gate": "delivery:verify", "body": "**ACTION NEEDED:** verify deploy: reply \"ok\"", "options": ["ok"]}))
        .await;
    let other_card = other["card"].as_str().unwrap().to_string();
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);

    let r = pr_event(&h, ready(HEAD)).await;

    assert_eq!(r["status"], "gated", "{r}");
    let open = cards(&h);
    assert_eq!(open.len(), 2, "both the unrelated ticket gate card and the merge card stay open: {open:?}");
    assert!(open.iter().any(|c| c["id"] == other_card), "{open:?}");
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("delivery:verify"));
}

#[tokio::test]
async fn a_waiting_ready_suppresses_idle_nudges_and_records_one_digest_line() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));

    assert_eq!(pr_event(&h, ready(HEAD)).await["status"], "waiting");
    h.app.wf.reconcile().await.unwrap();
    h.app.wf.reconcile().await.unwrap();
    let handled = digest(&h).await;
    assert_eq!(handled.len(), 1, "one waiting digest line across polls: {handled:?}");
    assert!(handled[0].contains("waiting (checks still running: test)"), "{}", handled[0]);

    h.tick(200).await;
    assert!(to(&h, WORKER).is_empty(), "worker waiting on reconcile must not be nudged: {:?}", to(&h, WORKER));
    let wl = h.app.store.link_get(WORKER).unwrap().unwrap();
    assert_eq!(wl.nudges, 0);
    assert!(!wl.escalated);
    assert!(
        !to(&h, TICKET).iter().any(|m| m.contains("escalate")),
        "waiting worker must not escalate to ticket: {:?}",
        to(&h, TICKET)
    );
}

#[tokio::test]
async fn an_adopting_ticket_retargets_card_metadata_and_releases_old_gate_even_when_busy() {
    let h = setup(merge_and_deploy());
    let mut t1 = thread(TICKET, "idle", T0);
    t1["link"] = json!("https://t3.local/old-ticket");
    h.fake.put(t1);
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    assert_eq!(pr_event(&h, ready(HEAD)).await["status"], "gated");
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("merge"));

    let heir = "mcp:heir";
    let mut t2 = thread(heir, "idle", T0);
    t2["link"] = json!("https://t3.local/new-ticket");
    h.fake.put(t2);
    h.wf(heir, "join", json!({"parent": BOARD, "label": "widgets#41 heir", "role": "ticket"})).await;
    h.wf(heir, "gate", json!({"open": "delivery:verify"})).await;

    h.wf(heir, "adopt", json!({"from": TICKET})).await;

    let open = cards(&h);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0]["ticket"], heir);
    assert_eq!(open[0]["label"], "widgets#41 heir");
    assert_eq!(open[0]["link"], "https://t3.local/new-ticket");
    assert_eq!(gate_of(&h, TICKET), None, "old ticket's merge gate must be released");
    assert_eq!(gate_of(&h, heir).as_deref(), Some("delivery:verify"));
}

#[tokio::test]
async fn summary_fence_check_requires_a_real_fenced_block_and_accepts_tildes() {
    let h = setup(json!({}));
    ticket_and_worker(&h, "1/1", Some(pr_format())).await;

    let inline = GOOD.replace("```text\nwf.pr\n  opened\n```", "Use ```inline``` backticks here.");
    open_pr(&h, HEAD, &inline, &["src/lib.rs"]);
    let r = pr_event(&h, opened(HEAD)).await;
    assert_eq!(r["missing"], json!(["a fenced block under Summary"]), "{r}");

    let line_start_inline = GOOD.replace("```text\nwf.pr\n  opened\n```", "```inline```");
    open_pr(&h, HEAD, &line_start_inline, &["src/lib.rs"]);
    let r = pr_event(&h, opened(HEAD)).await;
    assert_eq!(r["missing"], json!(["a fenced block under Summary"]), "{r}");

    let tilde =
        GOOD.replace("```text\nwf.pr\n  opened\n```", "~~~text\nwf.pr\n  opened\n~~~").replace("Refs #41", "Refs: #41");
    open_pr(&h, HEAD, &tilde, &["src/lib.rs"]);
    assert_eq!(pr_event(&h, opened(HEAD)).await["status"], "open");
}

#[tokio::test]
async fn short_sha_then_full_sha_keeps_wait_and_merge_card() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    pr_event(&h, opened(&HEAD[..7])).await;
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));
    assert_eq!(pr_event(&h, ready(&HEAD[..7])).await["status"], "waiting");

    let u = pr_event(&h, json!({"event": "updated", "repo": REPO, "pr": 7, "head": HEAD})).await;
    assert_eq!(u["status"], "recorded", "{u}");
    let pr = h.app.store.link_get(WORKER).unwrap().unwrap().pr.unwrap();
    assert!(pr.waiting && pr.head == HEAD, "{pr:?}");

    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS")]));
    h.app.wf.reconcile().await.unwrap();
    assert_eq!(cards(&h).len(), 1);
    assert_eq!(gate_of(&h, WORKER).as_deref(), Some("merge"));

    let u2 = pr_event(&h, json!({"event": "updated", "repo": REPO, "pr": 7, "head": &HEAD[..7]})).await;
    assert_eq!(u2["dropped"], json!([]));
    assert_eq!(cards(&h).len(), 1);
    assert_eq!(gate_of(&h, WORKER).as_deref(), Some("merge"));
}

#[tokio::test]
async fn plan_handles_ticket_paths_post_handoff_never_gate_and_omitted_chain_tail() {
    let h = setup(json!({"delivery": {"steps": [
        {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
        {"id": "lint", "skill": "verify", "gate": "never", "scope": "pr"},
        {"id": "publish", "skill": "publish", "gate": "chain", "scope": "ticket", "paths": ["docs/**"]},
        {"id": "announce", "skill": "deploy", "gate": "always", "scope": "post-handoff", "paths": ["docs/**"]}
    ]}}));
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);

    let r = pr_event(&h, ready(HEAD)).await;
    assert_eq!(r["options"], json!(["merge"]), "{r}");
    let card = cards(&h).remove(0);
    let body = card_body(&h, &card).await;
    assert!(body.contains("Later stages open their own gate: publish."), "{body}");
    assert!(!body.contains("lint"), "gate: never stage must not be listed as opening its own gate: {body}");
    assert!(!body.contains("Skipped for this PR"), "ticket/post-handoff stages must not be skipped on PR 1/2: {body}");

    let long = "a-stage-with-a-name-long-enough-to-pass-the-limit";
    let mut steps = vec![json!({"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"})];
    for i in 1..=8 {
        steps.push(json!({"id": format!("s{i}"), "skill": "deploy", "gate": "chain", "scope": "pr"}));
    }
    steps.push(json!({"id": long, "skill": "deploy", "gate": "chain", "scope": "pr"}));
    let h2 = setup(json!({"delivery": {"steps": steps}}));
    ticket_and_worker(&h2, "1/2", Some(pr_format())).await;
    open_pr(&h2, HEAD, GOOD, &["src/lib.rs"]);
    assert_eq!(pr_event(&h2, ready(HEAD)).await["status"], "gated");
    let body2 = card_body(&h2, &cards(&h2)[0]).await;
    assert!(body2.contains(&format!("Later stages open their own gate: s8, {long}.")), "{body2}");
}

#[tokio::test]
async fn a_waiting_pr_turned_draft_is_retried_immediately_and_superseded_refusal_is_not_sent() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));
    assert_eq!(pr_event(&h, ready(HEAD)).await["status"], "waiting");
    h.github.update_pr(REPO, 7, json!({"isDraft": true}));

    h.app.wf.reconcile().await.unwrap();
    let back = to(&h, WORKER);
    assert_eq!(back.len(), 1, "{back:?}");
    assert!(back[0].contains("is a draft"), "{}", back[0]);

    let h2 = setup(merge_and_deploy());
    ticket_and_worker(&h2, "1/2", Some(pr_format())).await;
    open_pr(&h2, HEAD, GOOD, &["src/lib.rs"]);
    h2.github.set_checks(REPO, 7, json!([pending_check("test")]));
    assert_eq!(pr_event(&h2, ready(HEAD)).await["status"], "waiting");
    h2.github.update_pr(REPO, 7, json!({"headRefOid": MOVED}));
    h2.github.set_delay(std::time::Duration::from_millis(200));
    let update = async {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        pr_event(&h2, json!({"event": "updated", "repo": REPO, "pr": 7, "head": MOVED})).await
    };
    let (rec, upd) = tokio::join!(h2.app.wf.reconcile(), update);
    rec.unwrap();
    assert_eq!(upd["status"], "recorded", "{upd}");
    assert!(to(&h2, WORKER).is_empty(), "superseded refusal must not be sent to worker: {:?}", to(&h2, WORKER));
}

#[tokio::test]
async fn settled_non_success_checks_and_unverified_fallbacks_report_accurately() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS"), check_run("lint", "CANCELLED")]));

    assert_eq!(pr_event(&h, ready(HEAD)).await["status"], "gated");
    let body = card_body(&h, &cards(&h)[0]).await;
    assert!(body.contains("Every check met merge policy at head"), "{body}");
    assert!(body.contains("lint (CANCELLED)"), "{body}");
    assert!(!body.contains("Every check passed"), "{body}");

    let h2 = setup(merge_and_deploy());
    ticket_and_worker(&h2, "1/2", Some(pr_format())).await;
    open_pr(&h2, HEAD, GOOD, &["src/lib.rs"]);
    h2.github.set_checks(REPO, 7, json!([]));
    assert_eq!(pr_event(&h2, ready(HEAD)).await["status"], "ticket");
    let woke = to(&h2, TICKET);
    assert!(!woke[0].contains("checks passed"), "unverified fallback must not claim checks passed: {}", woke[0]);
}

#[tokio::test]
async fn opened_adopted_mid_read_records_on_the_adopting_ticket() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    let heir = "mcp:heir";
    h.fake.put(thread(heir, "idle", T0));
    h.wf(heir, "join", json!({"parent": BOARD, "label": "widgets#41 heir", "role": "ticket"})).await;
    h.github.set_delay(std::time::Duration::from_millis(250));
    let adopt = async {
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        h.wf(heir, "adopt", json!({"from": TICKET})).await
    };

    let (r, _) = tokio::join!(pr_event(&h, opened(HEAD)), adopt);
    assert_eq!(r["status"], "open", "{r}");

    h.wf(WORKER, "send", json!({"kind": "event", "body": "wake heir"})).await;
    let woke_heir = to(&h, heir);
    assert_eq!(woke_heir.len(), 1, "{woke_heir:?}");
    assert!(woke_heir[0].contains("checked its description and Publishing line"), "{}", woke_heir[0]);
    assert!(to(&h, TICKET).is_empty());
}

#[tokio::test]
async fn a_waiting_ready_preserves_reported_config_dir_for_reconcile() {
    let h = setup(json!({}));
    let custom = h.dir.path().join("custom-claude");
    let d = custom.join("skills/custom-deploy");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("SKILL.md"), "---\nname: custom-deploy\ndescription: test\n---\n\nBody.\n").unwrap();
    let repo_cfg = h.dir.path().join("repo/.t3-workflow.json");
    std::fs::write(
        &repo_cfg,
        json!({"version": 1, "delivery": {"steps": [
            {"id": "merge", "kind": "builtin", "gate": "always", "scope": "pr"},
            {"id": "ship", "skill": "custom-deploy", "gate": "chain", "scope": "pr"}
        ]}})
        .to_string(),
    )
    .unwrap();
    let mut th = thread(WORKER, "idle", T0);
    th["projectId"] = json!("proj:widgets");
    th["providerInstanceId"] = json!("claude-custom");
    h.fake.put(th);
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));

    let rep =
        t3_codemode::wf_slots::Reported { own: Some(custom.to_string_lossy().into_owned()), ..Default::default() };
    let r = h.app.wf.dispatch_from("pr", Some(WORKER), &rep, ready(HEAD)).await.unwrap();
    assert_eq!(r["status"], "waiting", "{r}");

    h.github.set_checks(REPO, 7, json!([check_run("test", "SUCCESS")]));
    h.app.wf.reconcile().await.unwrap();
    let open = cards(&h);
    assert_eq!(open.len(), 1, "{open:?}");
    assert_eq!(open[0]["options"], json!(["merge and ship", "merge"]), "{open:?}");
}

#[test]
fn open_child_pr_schema_rejects_unknown_fields() {
    let entry = t3_codemode::catalog::builtins().into_iter().find(|e| e.name == "wf.openChild").unwrap();
    assert_eq!(entry.input_schema["properties"]["pr"]["additionalProperties"], json!(false));
}

#[tokio::test]
async fn an_unreadable_rollup_with_unknown_merge_state_reaches_the_ticket_fallback() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.set_checks(REPO, 7, json!([pending_check("test")]));
    assert_eq!(pr_event(&h, ready(HEAD)).await["status"], "waiting");

    h.github.set_checks(REPO, 7, json!([]));
    h.github.update_pr(REPO, 7, json!({"mergeStateStatus": "UNKNOWN"}));
    h.app.wf.reconcile().await.unwrap();

    let woke = to(&h, TICKET);
    assert_eq!(woke.len(), 1, "{woke:?}");
    assert!(woke[0].contains("cannot verify the checks"), "{}", woke[0]);
    assert!(!h.app.store.link_get(WORKER).unwrap().unwrap().pr.unwrap().waiting);
}

#[tokio::test]
async fn an_older_merge_wait_does_not_block_dropping_a_stale_ready_card() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    let third = "3333333333333333333333333333333333333333";
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    h.github.update_pr(REPO, 7, json!({"isMergeQueueEnabled": true, "mergeStateStatus": "BLOCKED"}));
    let m = h.wf(TICKET, "merge", json!({"repo": REPO, "pr": 7, "head": HEAD})).await;
    assert_eq!(m["status"], "waiting", "{m}");

    open_pr(&h, MOVED, GOOD, &["src/lib.rs"]);
    pr_event(&h, json!({"event": "updated", "repo": REPO, "pr": 7, "head": MOVED})).await;
    assert_eq!(pr_event(&h, ready(MOVED)).await["status"], "gated");
    assert_eq!(gate_of(&h, WORKER).as_deref(), Some("merge"));

    h.github.update_pr(REPO, 7, json!({"headRefOid": third}));
    h.app.wf.reconcile().await.unwrap();

    assert_eq!(gate_of(&h, WORKER), None, "worker merge hold for B must be released when PR moves to C");
    let back = to(&h, WORKER);
    assert_eq!(back.len(), 1, "{back:?}");
    assert!(back[0].contains("moved to 3333333 outside this thread"), "{}", back[0]);
}

#[tokio::test]
async fn clearing_a_delivery_gate_preserves_an_open_ready_merge_card_and_transfers_hold() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    let other = h
        .wf(
            TICKET,
            "send",
            json!({"kind": "gate", "gate": "delivery:verify", "body": "**ACTION NEEDED:** verify deploy: reply \"ok\"", "options": ["ok"]}),
        )
        .await;
    let other_card = other["card"].as_str().unwrap().to_string();
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    let ready_card = pr_event(&h, ready(HEAD)).await["card"].as_str().unwrap().to_string();
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("delivery:verify"));

    h.wf(TICKET, "gate", json!({"clear": true})).await;

    let open = cards(&h);
    assert_eq!(open.len(), 1, "only the delivery gate card should resolve: {open:?}");
    assert_eq!(open[0]["id"], ready_card);
    assert!(open.iter().all(|c| c["id"] != other_card));
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("merge"));
}

#[tokio::test]
async fn normalized_section_headings_merge_their_requirements() {
    let h = setup(json!({}));
    ticket_and_worker(
        &h,
        "1/1",
        Some(json!({"format": "custom", "sections": {"Notes": ["Alpha:"], " Notes ": ["Beta:"]}})),
    )
    .await;
    open_pr(&h, HEAD, "## Notes\n\nAlpha: yes\n\nRefs #41\n", &["src/lib.rs"]);

    let r = pr_event(
        &h,
        json!({"event": "opened", "repo": REPO, "pr": 7, "head": HEAD, "publishing": "Publishing: custom"}),
    )
    .await;

    assert_eq!(r["status"], "rewrite", "{r}");
    assert_eq!(r["missing"], json!(["\"Beta:\" under Notes"]), "{r}");
}

#[tokio::test]
async fn a_second_ticket_tracking_only_through_pr_seen_still_gets_the_merge_notice() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    assert_eq!(pr_event(&h, ready(HEAD)).await["status"], "gated");

    let tracker = "mcp:tracker";
    let tracker_worker = "mcp:tracker-worker";
    h.fake.put(thread(tracker, "idle", T0));
    h.fake.put(thread(tracker_worker, "idle", T0));
    h.wf(tracker, "join", json!({"parent": BOARD, "label": "widgets#99", "role": "ticket"})).await;
    h.wf(tracker, "openChild", json!({"role": "worker", "label": "#99 PR 1/1", "part": "1/1"})).await;
    h.wf(tracker_worker, "join", json!({"parent": tracker, "label": "#99 PR 1/1", "role": "worker"})).await;
    h.fake.set_prs(tracker_worker, vec![pull_request(REPO, 7, "open")]);
    h.tick(0).await;
    h.app.wf.reconcile().await.unwrap();
    let seen = h.app.store.pr_seen_get(REPO, 7).unwrap().unwrap();
    assert!(seen.tickets.iter().any(|(x, _)| x == tracker), "{seen:?}");
    h.wf(tracker, "close", json!({"child": tracker_worker})).await;

    let heir = "mcp:heir";
    h.fake.put(thread(heir, "idle", T0));
    h.wf(heir, "join", json!({"parent": BOARD, "label": "widgets#41 heir", "role": "ticket"})).await;
    h.github.update_pr(REPO, 7, json!({"state": "MERGED", "mergeCommit": {"oid": MOVED}}));
    h.github.set_delay(std::time::Duration::from_millis(300));
    let adopt = async {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        h.wf(heir, "adopt", json!({"from": TICKET})).await
    };
    let (r, _) = tokio::join!(h.app.wf.reconcile(), adopt);
    r.unwrap();

    assert!(
        to(&h, tracker).iter().any(|m| m.contains("merged on GitHub")),
        "second ticket tracked only via pr_seen must receive the merge notice: {:?}",
        to(&h, tracker)
    );
    assert!(
        to(&h, heir).iter().any(|m| m.contains("merged on GitHub")),
        "adopting ticket must receive the merge notice: {:?}",
        to(&h, heir)
    );
    assert!(to(&h, TICKET).is_empty(), "retiring ticket must not receive the merge notice: {:?}", to(&h, TICKET));
}

#[tokio::test]
async fn clearing_a_merge_gate_keeps_ready_merge_cards_for_other_prs_open() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    let worker2 = "mcp:worker-2";
    let mut th2 = thread(worker2, "idle", T0);
    th2["projectId"] = json!("proj:widgets");
    h.fake.put(th2);
    h.wf(
        TICKET,
        "openChild",
        json!({"role": "worker", "label": "#41 PR 2/2", "issue": "acme/widgets#41", "part": "2/2", "pr": pr_format()}),
    )
    .await;
    h.wf(worker2, "join", json!({"parent": TICKET, "label": "#41 PR 2/2", "role": "worker"})).await;

    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    let mut pr8 = github_pr(MOVED, "CLEAN", false);
    pr8["body"] = json!(GOOD);
    pr8["files"] = json!({"pageInfo": {"hasNextPage": false}, "nodes": [{"path": "src/lib.rs"}]});
    h.github.set_pr(REPO, 8, pr8);
    h.github.set_checks(REPO, 8, json!([check_run("test", "SUCCESS")]));

    let c1 = pr_event(&h, ready(HEAD)).await["card"].as_str().unwrap().to_string();
    let c2 = h.wf(worker2, "pr", json!({"event": "ready", "repo": REPO, "pr": 8, "head": MOVED})).await["card"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(cards(&h).len(), 2);
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("merge"));

    h.wf(TICKET, "gate", json!({"clear": true})).await;

    let open = cards(&h);
    assert_eq!(open.len(), 1, "only PR #7's merge card should resolve: {open:?}");
    assert_eq!(open[0]["id"], c2);
    assert!(open.iter().all(|c| c["id"] != c1));
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("merge"));

    h.wf(TICKET, "gate", json!({"clear": true})).await;
    assert!(cards(&h).is_empty());
    assert_eq!(gate_of(&h, TICKET), None);
}

#[tokio::test]
async fn answering_second_ready_merge_card_then_clearing_gate_keeps_first_open() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    let worker2 = "mcp:worker-2";
    let mut th2 = thread(worker2, "idle", T0);
    th2["projectId"] = json!("proj:widgets");
    h.fake.put(th2);
    h.wf(
        TICKET,
        "openChild",
        json!({"role": "worker", "label": "#41 PR 2/2", "issue": "acme/widgets#41", "part": "2/2", "pr": pr_format()}),
    )
    .await;
    h.wf(worker2, "join", json!({"parent": TICKET, "label": "#41 PR 2/2", "role": "worker"})).await;

    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);
    let mut pr8 = github_pr(MOVED, "CLEAN", false);
    pr8["body"] = json!(GOOD);
    pr8["files"] = json!({"pageInfo": {"hasNextPage": false}, "nodes": [{"path": "src/lib.rs"}]});
    h.github.set_pr(REPO, 8, pr8);
    h.github.set_checks(REPO, 8, json!([check_run("test", "SUCCESS")]));

    let c1 = pr_event(&h, ready(HEAD)).await["card"].as_str().unwrap().to_string();
    let c2 = h.wf(worker2, "pr", json!({"event": "ready", "repo": REPO, "pr": 8, "head": MOVED})).await["card"]
        .as_str()
        .unwrap()
        .to_string();

    // User answers the SECOND PR's merge card first, then the ticket clears its gate after handling it.
    h.app.wf.answer(&c2, "merge").await.unwrap();
    h.wf(TICKET, "gate", json!({"clear": true})).await;

    let open = cards(&h);
    assert_eq!(open.len(), 1, "unanswered PR #7 merge card must stay open: {open:?}");
    assert_eq!(open[0]["id"], c1);
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("merge"));
}

#[tokio::test]
async fn four_space_indented_backticks_do_not_count_as_summary_fence() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/1", Some(pr_format())).await;
    let bad = "## Summary\n\n    ```text\n    indented\n    ```\n\n## Evidence\n\n- **Before:** a\n  **After:** b\n\n\
## Merge Danger\n\n**Door:** two-way\n\n**Blast Radius:** small\n\nRefs #41\n";
    open_pr(&h, HEAD, bad, &["src/lib.rs"]);
    let out = pr_event(&h, opened(HEAD)).await;
    assert_eq!(out["status"], "rewrite", "{out}");
    assert!(out["message"].as_str().unwrap().contains("fenced block"), "{out}");
}

#[tokio::test]
async fn overseer_no_ci_fallback_does_not_claim_checks_passed() {
    let h = setup(merge_and_deploy());
    let boss = "mcp:boss";
    h.fake.put(thread(boss, "idle", T0));
    h.wf(boss, "join", json!({"parent": BOARD, "label": "overseer", "role": "overseer"})).await;
    h.wf(boss, "openChild", json!({"role": "ticket", "label": "widgets#41"})).await;
    h.wf(TICKET, "join", json!({"parent": boss, "label": "widgets#41", "role": "ticket"})).await;
    h.wf(TICKET, "openChild", json!({"role": "worker", "label": "#41 PR 1/1", "part": "1/1", "pr": pr_format()})).await;
    h.wf(WORKER, "join", json!({"parent": TICKET, "label": "#41 PR 1/1", "role": "worker"})).await;

    let mut op = opened(HEAD);
    op["checks"] = json!("none");
    pr_event(&h, op).await;
    let out = pr_event(&h, ready(HEAD)).await;
    assert_eq!(out["status"], "ticket", "{out}");
    let last = to(&h, TICKET).pop().unwrap();
    assert!(!last.contains("checks passed"), "no-CI fallback must not claim checks passed: {last}");
}

#[tokio::test]
async fn opening_a_delivery_gate_keeps_an_open_ready_merge_card_open() {
    let h = setup(merge_and_deploy());
    ticket_and_worker(&h, "1/2", Some(pr_format())).await;
    open_pr(&h, HEAD, GOOD, &["src/lib.rs"]);

    let merge_card = pr_event(&h, ready(HEAD)).await["card"].as_str().unwrap().to_string();
    assert_eq!(cards(&h).len(), 1);
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("merge"));

    let delivery_card = h
        .wf(
            TICKET,
            "send",
            json!({"kind": "gate", "gate": "delivery:deploy",
                   "body": "**ACTION NEEDED:** deploy PR #8: reply \"deploy\" or \"skip\"",
                   "options": ["deploy", "skip"]}),
        )
        .await["card"]
        .as_str()
        .unwrap()
        .to_string();

    let open = cards(&h);
    assert_eq!(open.len(), 2, "opening delivery:deploy must keep ready merge card open: {open:?}");
    assert!(open.iter().any(|c| c["id"] == merge_card), "{open:?}");
    assert!(open.iter().any(|c| c["id"] == delivery_card), "{open:?}");
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("delivery:deploy"));

    h.wf(TICKET, "gate", json!({"clear": true})).await;
    let remaining = cards(&h);
    assert_eq!(remaining.len(), 1, "clearing delivery gate must leave ready merge card open: {remaining:?}");
    assert_eq!(remaining[0]["id"], merge_card);
    assert_eq!(gate_of(&h, TICKET).as_deref(), Some("merge"));
}
