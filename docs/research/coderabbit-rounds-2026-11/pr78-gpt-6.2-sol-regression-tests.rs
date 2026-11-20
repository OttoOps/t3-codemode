#[tokio::test]
async fn review_reapproval_after_a_queue_removal_should_keep_the_wait() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    check(&h, 0).await;
    h.github.drop_from_queue(REPO, 7, "manual", &at(1));
    h.clock.set(&at(2));
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    check(&h, 3).await;

    assert_eq!(
        h.app.store.merge_waits().unwrap().len(),
        1,
        "cards: {:?}",
        cards(&h, false)
    );
}

#[tokio::test]
async fn review_callerless_queue_exit_with_partial_timeline_should_be_retried() {
    let h = H::new();
    h.github.set_pr(REPO, 7, github_pr(HEAD, "CLEAN", true));
    let args = json!({"repo": REPO, "pr": 7, "head": HEAD});
    assert_eq!(
        h.app.wf.dispatch("merge", None, args).await.unwrap()["status"],
        "queued"
    );
    h.github.update_pr(
        REPO,
        7,
        json!({"isInMergeQueue": false, "timelineItems": {"nodes": null}}),
    );
    check(&h, 1).await;
    h.github.drop_from_queue(REPO, 7, "manual", &at(1));
    check(&h, 2).await;

    assert_eq!(
        cards(&h, false).len(),
        1,
        "GitHub ops: {:?}",
        h.github.operations()
    );
}

#[tokio::test]
async fn review_queued_new_head_should_surface_the_mismatch_even_with_a_gate() {
    let h = H::new();
    ticket_with_worker(&h, 0).await;
    h.github.set_pr(REPO, 7, github_pr(HEAD, "BLOCKED", true));
    merge_gate(&h, Some(HEAD)).await;
    assert_eq!(wf_merge(&h, 7, HEAD).await["status"], "waiting");
    check(&h, 0).await;
    h.github.update_pr(
        REPO,
        7,
        json!({"headRefOid": MOVED, "isInMergeQueue": true}),
    );
    check(&h, 2).await;

    let open = cards(&h, false);
    assert!(
        open.iter().any(|c| c["kind"] == "report"
            && c["text"]
                .as_str()
                .unwrap()
                .contains("not at its approved head")),
        "{open:?}"
    );
}
