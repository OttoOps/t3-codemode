use crate::common::*;
use serde_json::{Value, json};
use t3_codemode::board::{
    BOARD, Issue, Remote, Tracker, parse_github_issue_only, parse_issue, parse_issue_only, parse_remote,
};
use t3_codemode::config::Config;
use t3_codemode::fake::{github_issue, project};

fn cards(h: &H) -> Vec<Value> {
    h.app.wf.cards().unwrap().into_iter().map(|c| serde_json::to_value(c).unwrap()).collect()
}

fn with_trackers(trackers: Value) -> H {
    H::build(Vec::new(), Config::from_value(json!({ "trackers": trackers })).unwrap())
}

#[test]
fn parses_github_gitlab_and_jira_refs() {
    // GitHub forms stay byte-identical
    let gh = parse_issue("https://github.com/Acme/widgets/issues/41#issuecomment-1").unwrap();
    assert_eq!(
        gh,
        Issue {
            tracker: Tracker::Github,
            host: Some("github.com".into()),
            scope: "Acme/widgets".into(),
            key: "41".into(),
            url: Some("https://github.com/Acme/widgets/issues/41".into()),
        }
    );
    assert_eq!(gh.label(), "widgets#41");
    assert_eq!(gh.slug(), "Acme/widgets#41");
    assert_eq!(gh.url(), Some("https://github.com/Acme/widgets/issues/41"));
    assert_eq!(gh.reference(), "https://github.com/Acme/widgets/issues/41");
    assert_eq!(gh.github_parts(), Some(("Acme", "widgets", 41)));

    let gh_short = parse_issue("Acme/widgets#41").unwrap();
    assert_eq!(gh_short, gh);
    assert!(parse_issue("https://github.com/Acme/widgets/pull/41").is_some());
    assert!(parse_issue_only("https://github.com/Acme/widgets/pull/41").is_none());
    assert!(parse_github_issue_only("Acme/widgets#41").is_some());

    // GitLab issue URLs on gitlab.com and self-hosted hosts with nested subgroups and ports
    let gl = parse_issue("https://gitlab.example.com/team/sub/widgets/-/issues/41#note_12").unwrap();
    assert_eq!(
        gl,
        Issue {
            tracker: Tracker::Gitlab,
            host: Some("gitlab.example.com".into()),
            scope: "team/sub/widgets".into(),
            key: "41".into(),
            url: Some("https://gitlab.example.com/team/sub/widgets/-/issues/41".into()),
        }
    );
    assert_eq!(gl.label(), "widgets#41");
    assert_eq!(gl.slug(), "gitlab.example.com/team/sub/widgets#41");
    assert_eq!(gl.reference(), "https://gitlab.example.com/team/sub/widgets/-/issues/41");
    assert_eq!(gl.github_parts(), None);
    assert_eq!(parse_issue_only(&gl.slug()), Some(gl.clone()));
    assert!(parse_github_issue_only("https://gitlab.example.com/team/sub/widgets/-/issues/41").is_none());

    let gl_port = parse_issue("http://gitlab.internal:8080/org/app/-/issues/9?tab=notes").unwrap();
    assert_eq!(gl_port.tracker, Tracker::Gitlab);
    assert_eq!(gl_port.host.as_deref(), Some("gitlab.internal"));
    assert_eq!(gl_port.scope, "org/app");
    assert_eq!(gl_port.key, "9");
    assert_eq!(gl_port.url(), Some("http://gitlab.internal:8080/org/app/-/issues/9"));
    assert_eq!(gl_port.slug(), "http://gitlab.internal:8080/org/app/-/issues/9");
    assert_eq!(parse_issue_only(&gl_port.slug()), Some(gl_port));

    let gl_8443 = parse_issue("https://gitlab.example:8443/team/app/-/issues/9#note_1").unwrap();
    let gl_9443 = parse_issue("https://gitlab.example:9443/team/app/-/issues/9?tab=notes").unwrap();
    assert_ne!(gl_8443.slug(), gl_9443.slug());
    assert_eq!(parse_issue_only(&gl_8443.slug()), Some(gl_8443));
    assert_eq!(parse_issue_only(&gl_9443.slug()), Some(gl_9443));

    // Rejects malformed GitLab URLs
    assert!(parse_issue("https://gitlab.example.com/org/app/issues/9").is_none());
    assert!(parse_issue("https://gitlab.example.com/onlyone/-/issues/9").is_none());
    assert!(parse_issue("https://gitlab.example.com/org/app/-/issues/0").is_none());
    assert!(parse_issue("https://gitlab.example.com/org/app/-/merge_requests/9").is_none());
    assert!(parse_issue("https://github.com/org/app/-/issues/9").is_none());
    assert!(parse_issue("https://user:pass@gitlab.example.com/org/app/-/issues/9").is_none());

    // Jira bare keys
    let jira = parse_issue("PROJ-123").unwrap();
    assert_eq!(
        jira,
        Issue { tracker: Tracker::Jira, host: None, scope: "PROJ".into(), key: "PROJ-123".into(), url: None }
    );
    assert_eq!(jira.label(), "PROJ-123");
    assert_eq!(jira.slug(), "PROJ-123");
    assert_eq!(jira.url(), None);
    assert_eq!(jira.reference(), "PROJ-123");
    assert_eq!(parse_issue_only("ENG2-42").unwrap().scope, "ENG2");
    assert_eq!(parse_issue_only("OPS_1-9").unwrap().key, "OPS_1-9");
    assert!(parse_github_issue_only("PROJ-123").is_none());

    // Rejects invalid Jira keys
    assert!(parse_issue("proj-123").is_none());
    assert!(parse_issue("PROJ-0").is_none());
    assert!(parse_issue("PROJ-01").is_none());
    assert!(parse_issue("P-1").is_none());
    assert!(parse_issue("123-456").is_none());
    assert!(parse_issue("PROJ-123-4").is_none());

    // Jira browse URLs (root and with context path)
    let jira_url = parse_issue("https://acme.atlassian.net/browse/PROJ-123?focusedCommentId=5").unwrap();
    assert_eq!(
        jira_url,
        Issue {
            tracker: Tracker::Jira,
            host: Some("acme.atlassian.net".into()),
            scope: "PROJ".into(),
            key: "PROJ-123".into(),
            url: Some("https://acme.atlassian.net/browse/PROJ-123".into()),
        }
    );
    assert_eq!(jira_url.label(), "PROJ-123");
    assert_eq!(jira_url.slug(), "acme.atlassian.net/PROJ-123");
    assert_eq!(jira_url.reference(), "https://acme.atlassian.net/browse/PROJ-123");
    assert_eq!(parse_issue_only(&jira_url.slug()), Some(jira_url.clone()));

    let jira_ctx = parse_issue("https://jira.acme.example:8443/jira/browse/proj-77/").unwrap();
    assert_eq!(jira_ctx.tracker, Tracker::Jira);
    assert_eq!(jira_ctx.host.as_deref(), Some("jira.acme.example"));
    assert_eq!((jira_ctx.scope.as_str(), jira_ctx.key.as_str()), ("PROJ", "PROJ-77"));
    assert_eq!(jira_ctx.url(), Some("https://jira.acme.example:8443/jira/browse/PROJ-77"));
    assert_eq!(jira_ctx.slug(), "https://jira.acme.example:8443/jira/browse/PROJ-77");
    assert_eq!(parse_issue_only(&jira_ctx.slug()), Some(jira_ctx.clone()));

    let jira_ctx_a = parse_issue("https://jira.acme.example/jira-a/browse/OPS-9").unwrap();
    let jira_ctx_b = parse_issue("https://jira.acme.example/jira-b/browse/OPS-9").unwrap();
    assert_ne!(jira_ctx_a.slug(), jira_ctx_b.slug());
    assert_eq!(parse_issue_only(&jira_ctx_a.slug()), Some(jira_ctx_a));
    assert_eq!(parse_issue_only(&jira_ctx_b.slug()), Some(jira_ctx_b));

    assert!(parse_issue("https://github.com/acme/browse/PROJ-123").is_none());
    assert!(parse_issue("https://jira.acme.example/browse/PROJ-0").is_none());
}

#[test]
fn parses_remotes_on_any_host() {
    for (url, want_host, want_path) in [
        ("git@github.com:Acme/WIDGETS.git", "github.com", "Acme/WIDGETS"),
        ("https://github.com/acme/widgets", "github.com", "acme/widgets"),
        ("git@gitlab.com:team/widgets.git", "gitlab.com", "team/widgets"),
        ("git@gitlab.acme.example:team/sub/widgets.git", "gitlab.acme.example", "team/sub/widgets"),
        ("https://gitlab.acme.example/team/sub/widgets.git", "gitlab.acme.example", "team/sub/widgets"),
        ("https://gitlab.acme.example:8443/team/sub/widgets.git", "gitlab.acme.example", "team/sub/widgets"),
        ("ssh://git@gitlab.acme.example:2222/team/sub/widgets.git", "gitlab.acme.example", "team/sub/widgets"),
    ] {
        assert_eq!(parse_remote(url), Some(Remote { host: want_host.into(), path: want_path.into() }), "{url}");
    }
    for bad in [
        "",
        "/src/widgets",
        "file:///src/widgets.git",
        "https://github.com/onlyone",
        "https://github.com/a/b/c",
        "https://gitlab.acme.example/onlyone.git",
        "git@gitlab.acme.example:../evil.git",
    ] {
        assert_eq!(parse_remote(bad), None, "{bad}");
    }
}

#[test]
fn trackers_config_validates_keys_hosts_and_targets() {
    let ok = Config::from_value(json!({
        "trackers": {
            "jira": { "PROJ": "my-service", "OPS_1": "p-infra" },
            "hosts": { "jira.acme.example": "my-service", "gitlab.acme.example": "p-gl" }
        }
    }))
    .unwrap();
    assert_eq!(ok.trackers.jira_project("proj"), Some(("PROJ", "my-service")));
    assert_eq!(ok.trackers.host_project("JIRA.ACME.EXAMPLE"), Some(("jira.acme.example", "my-service")));

    for (bad, needle) in [
        (json!({"jira": {"A": "p"}}), "Jira project key"),
        (json!({"jira": {"PROJ-1": "p"}}), "Jira project key"),
        (json!({"jira": {"PROJ": "a", "proj": "b"}}), "same Jira project key"),
        (json!({"jira": {"PROJ": "   "}}), "must name a T3 project"),
        (json!({"hosts": {"https://jira.example.com": "p"}}), "hostname without scheme"),
        (json!({"hosts": {"jira.example.com:8443": "p"}}), "hostname without scheme"),
        (json!({"hosts": {"jira.example.com/jira": "p"}}), "hostname without scheme"),
        (json!({"hosts": {"Jira.Example.Com": "a", "jira.example.com": "b"}}), "same host"),
        (json!({"hosts": {"github.com": "p"}}), "git origin remote"),
        (json!({"hosts": {"jira.example.com": ""}}), "must name a T3 project"),
    ] {
        let err = Config::from_value(json!({ "trackers": bad })).unwrap_err().to_string();
        assert!(err.contains(needle), "expected {needle:?} in {err:?}");
    }
}

#[tokio::test]
async fn launches_jira_ref_via_trackers_config_and_dedupes_browse_url() {
    let h = with_trackers(json!({
        "jira": { "PROJ": "Widgets App" }
    }));
    let mut widgets = project("p-widgets", "/src/widgets");
    widgets["title"] = json!("Widgets App");
    h.fake.set_projects(vec![widgets]);
    h.remotes.set("/src/widgets", "git@gitlab.acme.example:team/widgets.git");

    let out = h.exec("async () => wf.launchTicket({ issue: 'PROJ-123' })").await;
    let r = result(&out);
    assert_eq!(r["ok"], true);
    assert_eq!(r["label"], "PROJ-123");
    assert_eq!(r["projectId"], "p-widgets");
    assert_eq!(r["title"], "PROJ-123");

    let l = &h.fake.launched()[0];
    assert_eq!(l["projectId"], "p-widgets");
    let msg = l["message"].as_str().unwrap();
    assert!(msg.starts_with("Work PROJ-123 with the t3-ticket skill."), "{msg}");
    assert!(
        msg.contains(r#"wf.join({ parent: "board", label: "PROJ-123", role: "ticket", issue: "PROJ-123" })"#),
        "{msg}"
    );

    // Join call from the brief succeeds and stores the Jira issue identity
    let call = msg.lines().find(|line| line.starts_with("async () =>")).unwrap();
    h.exec_as("launch:1", call).await;
    let ln = h.app.store.link_get("launch:1").unwrap().unwrap();
    assert_eq!(ln.identity.issue.as_deref(), Some("PROJ-123"));

    // Relaunching the same Jira issue via its browse URL is blocked by board dedupe
    let dup = h.exec("async () => wf.launchTicket({ issue: 'https://acme.atlassian.net/browse/PROJ-123' })").await;
    assert!(err_text(&dup).contains("PROJ-123 is already on the board"), "{}", dup.text);
}

#[tokio::test]
async fn launches_jira_browse_url_with_jira_key_over_host_and_explicit_project_override() {
    let h = with_trackers(json!({
        "jira": { "OPS": "p-ops" },
        "hosts": { "jira.acme.example": "p-default" }
    }));
    h.fake.set_projects(vec![
        project("p-default", "/src/default"),
        project("p-ops", "/src/ops"),
        project("p-override", "/src/override"),
    ]);

    // Host fallback when the Jira project key is not explicitly mapped
    let out = h
        .exec("async () => wf.launchTicket({ issue: 'https://jira.acme.example/browse/ENG-7', title: 'Fix sync' })")
        .await;
    let r = result(&out);
    assert_eq!(r["projectId"], "p-default");
    assert_eq!(r["label"], "ENG-7");
    assert_eq!(r["title"], "ENG-7: Fix sync");
    let msg = h.fake.launched()[0]["message"].as_str().unwrap().to_string();
    assert!(msg.starts_with("Work https://jira.acme.example/browse/ENG-7 with the t3-ticket skill."), "{msg}");

    // Jira key mapping takes precedence over host mapping
    let out = h.exec("async () => wf.launchTicket({ issue: 'https://jira.acme.example/browse/OPS-9' })").await;
    let r = result(&out);
    assert_eq!(r["projectId"], "p-ops");
    assert_eq!(r["title"], "OPS-9: https://jira.acme.example/browse/OPS-9");

    // Explicit project overrides both trackers.jira and trackers.hosts
    let out = h.exec("async () => wf.launchTicket({ issue: 'OPS-10', project: 'p-override' })").await;
    assert_eq!(result(&out)["projectId"], "p-override");

    // Unmapped bare Jira key works with an explicit project
    let out = h.exec("async () => wf.launchTicket({ issue: 'UNMAPPED-5', project: 'p-override' })").await;
    let r = result(&out);
    assert_eq!(r["projectId"], "p-override");
    assert_eq!(r["label"], "UNMAPPED-5");
    assert!(h.fake.launched()[3]["message"].as_str().unwrap().starts_with("Work UNMAPPED-5 with the t3-ticket skill."));
}

#[tokio::test]
async fn launches_gitlab_url_by_origin_remote_or_host_mapping_and_disambiguates_labels() {
    let h = with_trackers(json!({
        "hosts": { "gitlab.fallback.example": "p-fallback" }
    }));
    h.fake.set_projects(vec![
        project("p-gh", "/src/gh-widgets"),
        project("p-gl", "/src/gl-widgets"),
        project("p-fallback", "/src/fallback"),
    ]);
    h.remotes.set("/src/gh-widgets", "git@github.com:acme/widgets.git");
    h.remotes.set("/src/gl-widgets", "ssh://git@gitlab.acme.example:2222/team/sub/widgets.git");

    // Launch GitHub widgets#41 first
    let gh = result(&h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#41' })").await);
    assert_eq!((&gh["label"], &gh["projectId"]), (&json!("widgets#41"), &json!("p-gh")));

    // Launch GitLab widgets#41 on self-hosted GitLab: matches origin remote and disambiguates label
    let gl = result(
        &h.exec("async () => wf.launchTicket({ issue: 'https://gitlab.acme.example/team/sub/widgets/-/issues/41' })")
            .await,
    );
    assert_eq!(gl["projectId"], "p-gl");
    assert_eq!(gl["label"], "gitlab.acme.example/team/sub/widgets#41");
    let msg = h.fake.launched()[1]["message"].as_str().unwrap().to_string();
    assert!(
        msg.starts_with("Work https://gitlab.acme.example/team/sub/widgets/-/issues/41 with the t3-ticket skill."),
        "{msg}"
    );
    let call = msg.lines().find(|line| line.starts_with("async () =>")).unwrap();
    h.exec_as("launch:2", call).await;
    let ln = h.app.store.link_get("launch:2").unwrap().unwrap();
    assert_eq!(ln.identity.issue.as_deref(), Some("gitlab.acme.example/team/sub/widgets#41"));

    // Relaunching the same GitLab issue is rejected by dedupe
    let dup = h
        .exec("async () => wf.launchTicket({ issue: 'https://gitlab.acme.example/TEAM/sub/widgets/-/issues/41' })")
        .await;
    assert!(err_text(&dup).contains("already on the board"), "{}", dup.text);

    // GitLab host falls back to trackers.hosts when no origin remote matches
    let fb = result(
        &h.exec("async () => wf.launchTicket({ issue: 'https://gitlab.fallback.example/org/other/-/issues/3' })").await,
    );
    assert_eq!((&fb["projectId"], &fb["label"]), (&json!("p-fallback"), &json!("other#3")));
}

#[tokio::test]
async fn unmapped_refs_fail_with_clear_remediation_and_missing_mapped_project_fails_clearly() {
    let h = with_trackers(json!({
        "jira": { "BROKEN": "no-such-project" }
    }));
    h.fake.set_projects(vec![project("p-widgets", "/src/widgets")]);
    h.remotes.set("/src/widgets", "git@github.com:acme/widgets.git");

    // Unmapped bare Jira key names both trackers.jira and explicit project
    let out = h.exec("async () => wf.launchTicket({ issue: 'PROJ-123' })").await;
    let err = err_text(&out);
    assert!(err.contains("trackers.jira"), "{err}");
    assert!(err.contains("explicit project"), "{err}");

    // Unmapped Jira browse URL names trackers.jira, trackers.hosts, and explicit project
    let out = h.exec("async () => wf.launchTicket({ issue: 'https://jira.acme.example/browse/PROJ-124' })").await;
    let err = err_text(&out);
    assert!(err.contains("trackers.jira"), "{err}");
    assert!(err.contains("trackers.hosts"), "{err}");
    assert!(err.contains("explicit project"), "{err}");

    // Unmapped GitLab issue names trackers.hosts and explicit project
    let out = h.exec("async () => wf.launchTicket({ issue: 'https://gitlab.acme.example/org/app/-/issues/8' })").await;
    let err = err_text(&out);
    assert!(err.contains("trackers.hosts"), "{err}");
    assert!(err.contains("explicit project"), "{err}");

    // Configured mapping pointing to a non-existent T3 project fails clearly
    let out = h.exec("async () => wf.launchTicket({ issue: 'BROKEN-1' })").await;
    let err = err_text(&out);
    assert!(err.contains("trackers.jira.BROKEN"), "{err}");
    assert!(err.contains("no-such-project"), "{err}");

    assert!(h.fake.launched().is_empty());
    assert_eq!(cards(&h).len(), 4);
}

#[tokio::test]
async fn wf_issue_stays_github_only_while_open_child_accepts_jira_and_gitlab() {
    let h = H::new();
    let out = h.exec_as(WORKER, "async () => wf.issue({ url: 'PROJ-123' })").await;
    assert!(err_text(&out).contains("is not a GitHub issue"), "{}", out.text);
    let out =
        h.exec_as(WORKER, "async () => wf.issue({ url: 'https://gitlab.acme.example/org/app/-/issues/5' })").await;
    assert!(err_text(&out).contains("is not a GitHub issue"), "{}", out.text);

    // openChild / join carry Jira and GitLab issue & parentIssue identities cleanly
    h.wf(TICKET, "init", json!({"role": "ticket", "issue": "PROJ-123"})).await;
    let opened = h
        .wf(
            TICKET,
            "openChild",
            json!({
                "role": "worker",
                "label": "PROJ-124 PR 1/2",
                "issue": "https://jira.acme.example/browse/PROJ-124",
                "parentIssue": "PROJ-123",
                "part": "1/2"
            }),
        )
        .await;
    let brief = opened["brief"].as_str().unwrap();
    assert!(brief.contains(r#"issue: "jira.acme.example/PROJ-124", parentIssue: "PROJ-123", part: "1/2""#), "{brief}");
    h.wf(
        WORKER,
        "join",
        json!({
            "parent": TICKET,
            "label": "PROJ-124 PR 1/2",
            "role": "worker",
            "issue": "jira.acme.example/PROJ-124",
            "parentIssue": "PROJ-123",
            "part": "1/2"
        }),
    )
    .await;
    let ln = h.app.store.link_get(WORKER).unwrap().unwrap();
    assert_eq!(ln.identity.issue.as_deref(), Some("jira.acme.example/PROJ-124"));
    assert_eq!(ln.identity.parent_issue.as_deref(), Some("PROJ-123"));
    assert_eq!(ln.identity.part.as_deref(), Some("1/2"));
}

#[tokio::test]
async fn jira_urls_on_different_sites_do_not_collide_while_bare_key_dedupes_mapped_project() {
    let mut h = with_trackers(json!({
        "jira": { "OPS": "p-a" },
        "hosts": {
            "jira-a.example.com": "p-a",
            "jira-b.example.com": "p-b"
        }
    }));
    h.fake.set_projects(vec![project("p-a", "/src/a"), project("p-b", "/src/b")]);

    let first =
        result(&h.exec("async () => wf.launchTicket({ issue: 'https://jira-a.example.com/browse/OPS-9' })").await);
    assert_eq!((&first["projectId"], &first["label"]), (&json!("p-a"), &json!("OPS-9")));

    // Different Jira site with the same key launches cleanly with a host-qualified label
    let second = result(
        &h.exec("async () => wf.launchTicket({ issue: 'https://jira-b.example.com/browse/OPS-9', project: 'p-b' })")
            .await,
    );
    assert_eq!((&second["projectId"], &second["label"]), (&json!("p-b"), &json!("jira-b.example.com/OPS-9")));

    // Same hostname with a different context path (including case-only differences) launches cleanly without colliding
    let third = result(
        &h.exec(
            "async () => wf.launchTicket({ issue: 'https://jira-a.example.com/Jira/browse/OPS-9', project: 'p-b' })",
        )
        .await,
    );
    assert_eq!(
        (&third["projectId"], &third["label"]),
        (&json!("p-b"), &json!("https://jira-a.example.com/Jira/browse/OPS-9"))
    );
    let fourth = result(
        &h.exec(
            "async () => wf.launchTicket({ issue: 'https://jira-a.example.com/jira/browse/OPS-9', project: 'p-b' })",
        )
        .await,
    );
    assert_eq!(
        (&fourth["projectId"], &fourth["label"]),
        (&json!("p-b"), &json!("https://jira-a.example.com/jira/browse/OPS-9"))
    );
    // Same context path with different host/key case is still deduped after normalization
    let same_ctx = h
        .exec("async () => wf.launchTicket({ issue: 'https://JIRA-A.EXAMPLE.COM/jira/browse/ops-9', project: 'p-b' })")
        .await;
    assert!(err_text(&same_ctx).contains("was launched and has not joined yet"), "{}", same_ctx.text);

    // Bare key mapped to p-a is deduped against the pending ticket in p-a even after a restart clears live cache
    h.reopen();
    let dup = h.exec("async () => wf.launchTicket({ issue: 'OPS-9' })").await;
    assert!(err_text(&dup).contains("OPS-9 was launched and has not joined yet"), "{}", dup.text);

    // Bare key explicitly routed to a different project (p-c) launches with a distinct allocated label
    h.fake.set_projects(vec![project("p-a", "/src/a"), project("p-b", "/src/b"), project("p-c", "/src/c")]);
    let bare_other = result(&h.exec("async () => wf.launchTicket({ issue: 'OPS-9', project: 'p-c' })").await);
    assert_eq!((&bare_other["projectId"], &bare_other["label"]), (&json!("p-c"), &json!("OPS-9 (2)")));
    // Relaunching bare OPS-9 in p-c is still deduped, while routing bare OPS-9 to a fourth project succeeds
    let dup_c = h.exec("async () => wf.launchTicket({ issue: 'OPS-9', project: 'p-c' })").await;
    assert!(err_text(&dup_c).contains("OPS-9 (2) was launched and has not joined yet"), "{}", dup_c.text);
    h.fake.set_projects(vec![
        project("p-a", "/src/a"),
        project("p-b", "/src/b"),
        project("p-c", "/src/c"),
        project("p-d", "/src/d"),
    ]);
    let bare_d = result(&h.exec("async () => wf.launchTicket({ issue: 'OPS-9', project: 'p-d' })").await);
    assert_eq!((&bare_d["projectId"], &bare_d["label"]), (&json!("p-d"), &json!("OPS-9 (3)")));
}

#[tokio::test]
async fn gitlab_instances_on_different_web_ports_match_their_own_https_origin_and_do_not_collide() {
    let h = H::new();
    h.fake.set_projects(vec![project("p-8443", "/src/gl-8443"), project("p-9443", "/src/gl-9443")]);
    h.remotes.set("/src/gl-8443", "https://gitlab.example:8443/team/app.git");
    h.remotes.set("/src/gl-9443", "https://gitlab.example:9443/team/app.git");

    let first = result(
        &h.exec("async () => wf.launchTicket({ issue: 'https://gitlab.example:9443/team/app/-/issues/9' })").await,
    );
    assert_eq!((&first["projectId"], &first["label"]), (&json!("p-9443"), &json!("app#9")));

    let second = result(
        &h.exec("async () => wf.launchTicket({ issue: 'https://gitlab.example:8443/team/app/-/issues/9' })").await,
    );
    assert_eq!(
        (&second["projectId"], &second["label"]),
        (&json!("p-8443"), &json!("https://gitlab.example:8443/team/app/-/issues/9"))
    );
}

#[tokio::test]
async fn gitlab_ticket_does_not_block_github_wf_issue_or_flow() {
    let cfg = Config::from_value(json!({
        "flow": {
            "repos": ["acme/widgets"],
            "optIn": { "label": "flow" },
            "maxTickets": 2,
            "launch": "ask"
        }
    }))
    .unwrap();
    let h = H::build(Vec::new(), cfg);
    h.github.add_repo("acme/widgets");
    h.github.set_issue("acme/widgets", github_issue(41, "GH issue", &["flow"], "2026-10-01T09:00:00Z"));
    h.fake.set_projects(vec![project("p-gl", "/src/gl-widgets"), project("p-gh", "/src/gh-widgets")]);
    h.remotes.set("/src/gl-widgets", "git@gitlab.acme.example:team/widgets.git");
    h.remotes.set("/src/gh-widgets", "git@github.com:acme/widgets.git");

    // Launch a GitLab ticket labeled widgets#41
    let gl = result(
        &h.exec("async () => wf.launchTicket({ issue: 'https://gitlab.acme.example/team/widgets/-/issues/41' })").await,
    );
    assert_eq!(gl["label"], "widgets#41");

    // Reporting GitHub acme/widgets#41 via wf.issue still succeeds and stays visible in new_issues
    let reported = result(
        &h.exec_as(WORKER, "async () => wf.issue({ url: 'https://github.com/acme/widgets/issues/41', title: 'GH' })")
            .await,
    );
    assert_eq!(reported["added"], true, "{reported}");
    let (open_new, _) = h.app.wf.new_issues().unwrap();
    assert_eq!(open_new.len(), 1, "{open_new:?}");

    // Launching GitHub acme/widgets#41 while GitLab widgets#41 is open uses fallback label acme/widgets#41;
    // wf.issue now sees the GitHub ticket and new_issues hides it.
    let gh = result(&h.exec("async () => wf.launchTicket({ issue: 'acme/widgets#41' })").await);
    assert_eq!(gh["label"], "acme/widgets#41");
    let dup_report =
        result(&h.exec_as(WORKER, "async () => wf.issue({ url: 'https://github.com/acme/widgets/issues/41' })").await);
    assert_eq!(dup_report["added"], false, "{dup_report}");
    assert_eq!(dup_report["reason"], "ticket", "{dup_report}");
    let (open_new, _) = h.app.wf.new_issues().unwrap();
    assert!(open_new.is_empty(), "{open_new:?}");

    // Join and close both tickets; closing the fallback-labeled GitHub ticket moves acme/widgets#41 to skipped.
    h.wf(
        "launch:1",
        "join",
        json!({
            "parent": BOARD,
            "label": "widgets#41",
            "role": "ticket",
            "issue": "gitlab.acme.example/team/widgets#41"
        }),
    )
    .await;
    h.wf("launch:1", "send", json!({"kind": "handoff", "body": "done"})).await;

    h.wf(
        "launch:2",
        "join",
        json!({
            "parent": BOARD,
            "label": "acme/widgets#41",
            "role": "ticket",
            "issue": "acme/widgets#41"
        }),
    )
    .await;
    h.wf("launch:2", "send", json!({"kind": "handoff", "body": "done"})).await;

    let f = result(&h.exec("async () => wf.flow()").await);
    assert!(f["queue"].as_array().unwrap().is_empty(), "{f}");
    let skipped: Vec<&str> = f["skipped"].as_array().unwrap().iter().filter_map(|i| i["label"].as_str()).collect();
    assert_eq!(skipped, ["widgets#41"], "{f}");
}

#[tokio::test]
async fn closed_gitlab_ticket_alone_does_not_skip_github_flow_issue() {
    let cfg = Config::from_value(json!({
        "flow": {
            "repos": ["acme/widgets"],
            "optIn": { "label": "flow" },
            "maxTickets": 2,
            "launch": "ask"
        }
    }))
    .unwrap();
    let h = H::build(Vec::new(), cfg);
    h.github.add_repo("acme/widgets");
    h.github.set_issue("acme/widgets", github_issue(41, "GH issue", &["flow"], "2026-10-01T09:00:00Z"));
    h.fake.set_projects(vec![project("p-gl", "/src/gl-widgets")]);
    h.remotes.set("/src/gl-widgets", "git@gitlab.acme.example:team/widgets.git");

    result(
        &h.exec("async () => wf.launchTicket({ issue: 'https://gitlab.acme.example/team/widgets/-/issues/41' })").await,
    );
    h.wf(
        "launch:1",
        "join",
        json!({
            "parent": BOARD,
            "label": "widgets#41",
            "role": "ticket",
            "issue": "gitlab.acme.example/team/widgets#41"
        }),
    )
    .await;
    h.wf("launch:1", "send", json!({"kind": "handoff", "body": "done"})).await;

    let f = result(&h.exec("async () => wf.flow()").await);
    let queued: Vec<&str> = f["queue"].as_array().unwrap().iter().filter_map(|i| i["label"].as_str()).collect();
    assert_eq!(queued, ["widgets#41"], "{f}");
    assert!(f["skipped"].as_array().unwrap().is_empty(), "{f}");
}

#[tokio::test]
async fn api_launch_accepts_optional_project_per_ref_and_keeps_string_refs() {
    let h = with_trackers(json!({ "jira": { "ENG": "p-eng" } }));
    let s = crate::board::start(h).await;
    let mut fe = project("p-fe", "/src/fe");
    fe["title"] = json!("Frontend");
    s.h.fake.set_projects(vec![
        project("p-gh", "/src/gh-widgets"),
        project("p-gl", "/src/gl-service"),
        project("p-eng", "/src/eng"),
        fe,
    ]);
    s.h.remotes.set("/src/gh-widgets", "git@github.com:acme/widgets.git");
    s.h.remotes.set("/src/gl-service", "git@gitlab.acme.example:team/service.git");

    let r = s
        .post(
            "/api/launch",
            json!({
                "issues": [
                    "acme/widgets#1",
                    { "issue": "PROJ-123", "project": "p-gl" },
                    { "issue": "OPS-9", "project": "Frontend" },
                    { "issue": "ENG-5", "project": null },
                    { "issue": "  ", "project": "p-gl" }
                ]
            }),
            Some(&s.csrf()),
            None,
        )
        .await;
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    let results = body["results"].as_array().unwrap();
    assert_eq!(results.len(), 4, "{body}");
    assert_eq!(
        (&results[0]["ok"], &results[0]["projectId"], &results[0]["issue"]),
        (&json!(true), &json!("p-gh"), &json!("acme/widgets#1"))
    );
    assert_eq!(
        (&results[1]["ok"], &results[1]["projectId"], &results[1]["label"]),
        (&json!(true), &json!("p-gl"), &json!("PROJ-123"))
    );
    assert_eq!(
        (&results[2]["ok"], &results[2]["projectId"], &results[2]["label"]),
        (&json!(true), &json!("p-fe"), &json!("OPS-9"))
    );
    assert_eq!(
        (&results[3]["ok"], &results[3]["projectId"], &results[3]["label"]),
        (&json!(true), &json!("p-eng"), &json!("ENG-5"))
    );

    // Unknown fields inside an item or on the body are rejected with 400
    let bad_item = s
        .post("/api/launch", json!({"issues": [{"issue": "PROJ-2", "projectId": "p-gl"}]}), Some(&s.csrf()), None)
        .await;
    assert_eq!(bad_item.status(), 400);
    let bad_body = s.post("/api/launch", json!({"issues": ["PROJ-2"], "extra": true}), Some(&s.csrf()), None).await;
    assert_eq!(bad_body.status(), 400);
    let empty = s.post("/api/launch", json!({"issues": [{"issue": "   "}]}), Some(&s.csrf()), None).await;
    assert_eq!(empty.status(), 400);
    let too_many: Vec<Value> = (1..=21).map(|n| json!({"issue": format!("PROJ-{n}"), "project": "p-gl"})).collect();
    let over = s.post("/api/launch", json!({"issues": too_many}), Some(&s.csrf()), None).await;
    assert_eq!(over.status(), 400);
}

#[tokio::test]
async fn api_models_exposes_active_projects_remotes_and_trackers() {
    let h = with_trackers(json!({
        "jira": { "MAPPED": "p-api" },
        "hosts": { "jira.acme.example": "p-web" }
    }));
    let s = crate::board::start(h).await;
    let mut p_web = project("p-web", "/src/web");
    p_web["title"] = json!("Web App");
    let mut p_api = project("p-api", "/src/api");
    p_api["title"] = json!("API Service");
    let mut p_plain = project("p-plain", "/src/plain");
    p_plain["title"] = json!("Scratch");
    let mut p_arch = project("p-arch", "/src/arch");
    p_arch["title"] = json!("Archived");
    p_arch["archivedAt"] = json!("2026-11-02T00:00:00Z");
    let mut p_del = project("p-del", "/src/del");
    p_del["title"] = json!("Deleted");
    p_del["deletedAt"] = json!("2026-11-02T00:00:00Z");
    s.h.fake.set_projects(vec![p_web, p_api, p_plain, p_arch, p_del]);
    s.h.remotes.set("/src/web", "git@github.com:acme/web.git");
    s.h.remotes.set("/src/api", "https://token:secret@gitlab.acme.example:8443/team/api.git");

    let body = s.get("/api/models").await;
    assert_eq!(
        body["projects"],
        json!([
            { "id": "p-api", "title": "API Service", "remote": { "host": "gitlab.acme.example", "path": "team/api", "port": 8443 } },
            { "id": "p-plain", "title": "Scratch", "remote": null },
            { "id": "p-web", "title": "Web App", "remote": { "host": "github.com", "path": "acme/web", "port": null } }
        ]),
        "{body}"
    );
    assert_eq!(
        body["trackers"],
        json!({ "jira": { "MAPPED": "p-api" }, "hosts": { "jira.acme.example": "p-web" } }),
        "{body}"
    );
    let raw = body.to_string();
    assert!(!raw.contains("/src/"), "workspace paths must not be exposed: {raw}");
    assert!(!raw.contains("secret"), "remote credentials must not be exposed: {raw}");
}

#[tokio::test]
async fn launch_form_shows_project_picker_for_unresolved_non_github_refs_and_submits_picks() {
    let h = with_trackers(json!({ "jira": { "MAPPED": "API Service" } }));
    let s = crate::board::start(h).await;
    let lms_dir = s.h.dir.path().join("lms");
    std::fs::create_dir_all(&lms_dir).unwrap();
    std::fs::write(lms_dir.join("AGENTS.md"), "## Agent workflow\n\n- Ticket model: claude-opus-5-5, low effort\n")
        .unwrap();
    let mut p_gh = project("p-gh", "/src/gh");
    p_gh["title"] = json!("Widgets");
    let mut p_api = project("p-api", "/src/api");
    p_api["title"] = json!("API Service");
    let mut p_lms = project("p-lms", &lms_dir.to_string_lossy());
    p_lms["title"] = json!("LMS");
    s.h.fake.set_projects(vec![p_gh, p_api, p_lms]);
    s.h.remotes.set("/src/gh", "git@github.com:acme/widgets.git");
    s.h.remotes.set("/src/api", "https://gitlab.acme.example:8443/team/api.git");
    s.h.remotes.set(&lms_dir.to_string_lossy(), "git@gitlab.internal:team/lms.git");

    let html = s.http.get(format!("{}/", s.base)).send().await.unwrap().text().await.unwrap();
    let st: Value = s.get("/api/state").await;
    let models: Value = s.get("/api/models").await;
    let models_err = json!({
        "ticket": models["ticket"],
        "ask": models["ask"],
        "providers": models["providers"],
        "pools": models["pools"],
        "projects": [],
        "trackers": models["trackers"],
        "repoOverrides": [],
        "capabilitiesError": null,
        "projectsError": "t3 down",
        "modelsRev": 1,
        "modelsSession": models["modelsSession"],
    });

    let r = crate::board::run_page_async_with(
        &html,
        "",
        &format!(
            r##"async () => {{
const base = {st};
let curModels = {models};
const errModels = {models_err};
const tick = async () => {{ for (let i = 0; i < 8; i++) await Promise.resolve(); }};
fetchReply = (url, opts) => {{
  const m = (opts && opts.method) || "GET";
  if (url === "/api/models" && m === "GET") return Promise.resolve({{ ok: true, json: async () => curModels }});
  if (url === "/api/launch" && m === "POST") {{
    const body = JSON.parse(opts.body);
    return Promise.resolve({{
      ok: true,
      json: async () => ({{
        results: body.issues.map((item, i) => {{
          const issue = typeof item === "string" ? item : item.issue;
          return {{ ok: true, issue, label: issue, link: "https://t3/launch:" + (i + 1), model: "claude-opus-5-5", instanceId: "claudeAgent", modelSource: "default" }};
        }})
      }})
    }});
  }}
  return Promise.resolve({{ ok: true, json: async () => ({{}}) }});
}};
emitState(base);
await refreshModels(true);

const ta = document.getElementById("issues");
const box = document.getElementById("launch-projects");
const form = document.getElementById("launch");

// 1. GitHub refs (matched or unmatched) never show the project picker and submit plain strings.
ta.value = "acme/widgets#41\nacme/other#9";
ta.dispatch("input");
const ghHidden = box.hidden;
form.dispatch("submit");
await tick();
const ghPost = JSON.parse(fetches.filter((f) => f.url === "/api/launch")[0].body);

// 2. Mapped Jira key and origin-matched GitLab URL resolve automatically with no picker.
ta.value = "MAPPED-7\nhttps://gitlab.acme.example:8443/team/api/-/issues/9";
ta.dispatch("input");
const mappedHidden = box.hidden;

// 3. Unmapped Jira key + unmapped GitLab URL + GitHub ref: picker shows only the two unresolved refs.
ta.value = "acme/widgets#41\nPROJ-123\nhttps://gitlab.other.example/org/app/-/issues/4";
ta.dispatch("input");
const shownForUnmapped = !box.hidden;
const refsShown = [...box.querySelectorAll("select[data-ref]")].map((s) => s.dataset.ref);
const optsFirst = [...box.querySelectorAll("select[data-ref]")[0].children].map((o) => [o.getAttribute("value"), o.textContent]);

// Submitting before picking blocks POST, shows inline error, and marks selects invalid.
const postsBefore = fetches.filter((f) => f.url === "/api/launch").length;
form.dispatch("submit");
await tick();
const postsBlocked = fetches.filter((f) => f.url === "/api/launch").length;
const blockErr = box.querySelector("#launch-projects-err").textContent;
const firstInvalid = box.querySelectorAll("select[data-ref]")[0].getAttribute("aria-invalid");

// Picking LMS for PROJ-123 updates the launch model preview with LMS's AGENTS.md override.
const sels = box.querySelectorAll("select[data-ref]");
sels[0].value = "p-lms";
sels[0].dispatch("change");
const modelAfterPick = document.getElementById("launch-model-text").textContent;
sels[1].value = "p-api";
sels[1].dispatch("change");

// Submit succeeds with per-ref projects and clears the form + hides the picker.
form.dispatch("submit");
await tick();
const launchPosts = fetches.filter((f) => f.url === "/api/launch").map((f) => JSON.parse(f.body));
const clearedAfter = ta.value === "" && box.hidden;

// 4. Projects error state shows message and Retry button re-fetches /api/models.
curModels = errModels;
await refreshModels(true);
ta.value = "PROJ-55";
ta.dispatch("input");
const errText = box.textContent;
const modelGetsBefore = fetches.filter((f) => f.url === "/api/models").length;
box.querySelector("#launch-projects-refresh").dispatch("click");
await tick();
const modelGetsAfter = fetches.filter((f) => f.url === "/api/models").length;

return {{
  ghHidden, ghPost, mappedHidden, shownForUnmapped, refsShown, optsFirst,
  postsBefore, postsBlocked, blockErr, firstInvalid, modelAfterPick,
  mixedPost: launchPosts[1], clearedAfter, errText,
  retried: modelGetsAfter > modelGetsBefore
}};
}}"##
        ),
    );

    assert_eq!(r["ghHidden"], true, "{r}");
    assert_eq!(r["ghPost"], json!({"issues": ["acme/widgets#41", "acme/other#9"]}), "{r}");
    assert_eq!(r["mappedHidden"], true, "{r}");
    assert_eq!(r["shownForUnmapped"], true, "{r}");
    assert_eq!(r["refsShown"], json!(["PROJ-123", "https://gitlab.other.example/org/app/-/issues/4"]), "{r}");
    assert_eq!(
        r["optsFirst"],
        json!([["", "Choose a project..."], ["p-api", "API Service"], ["p-lms", "LMS"], ["p-gh", "Widgets"]]),
        "{r}"
    );
    assert_eq!(r["postsBefore"], r["postsBlocked"], "must not POST while picks are missing: {r}");
    assert!(r["blockErr"].as_str().unwrap().contains("PROJ-123"), "{r}");
    assert!(r["blockErr"].as_str().unwrap().contains("app#4"), "{r}");
    assert_eq!(r["firstInvalid"], "true", "{r}");
    assert!(r["modelAfterPick"].as_str().unwrap().contains("LMS"), "{r}");
    assert_eq!(
        r["mixedPost"],
        json!({
            "issues": [
                "acme/widgets#41",
                { "issue": "PROJ-123", "project": "p-lms" },
                { "issue": "https://gitlab.other.example/org/app/-/issues/4", "project": "p-api" }
            ]
        }),
        "{r}"
    );
    assert_eq!(r["clearedAfter"], true, "{r}");
    assert!(r["errText"].as_str().unwrap().contains("Could not load T3 projects: t3 down"), "{r}");
    assert_eq!(r["retried"], true, "{r}");
}
