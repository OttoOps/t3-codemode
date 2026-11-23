use std::collections::BTreeMap;

use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::board::{BOARD, Card, NewCard, Tracker, checked_options, gh_name, parse_issue_only};
use crate::github::errors;
use crate::merge::{CHECKS, CheckState, check_state};
use crate::reconcile::{CardPr, CardStage};
use crate::slots::{Caps, glob_matches};
use crate::store::{Link, LinkPr, PrBrief};
use crate::wf::{HANDLED, NO_CALLER, SendArgs, WATCHER, Wf, header};
use crate::wf_slots::Reported;

const PR_READ: &str = "query($owner: String!, $name: String!, $number: Int!) { repository(owner: $owner, name: $name) { \
pullRequest(number: $number) { state isDraft headRefOid mergeStateStatus reviewDecision body \
latestOpinionatedReviews(first: 100) { nodes { state author { login } commit { oid } } } \
files(first: 100) { pageInfo { hasNextPage } nodes { path } } ...Checks } } }";
const PR_FORMAT: &[(&str, &[&str])] =
    &[("Summary", &["```"]), ("Evidence", &["Before:", "After:"]), ("Merge Danger", &["Door:", "Blast Radius:"])];
const MAX_OPTIONS: usize = 8;
const MAX_OPTION_CHARS: usize = 60;
const FENCE: &str = "```";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PrArgs {
    event: String,
    repo: String,
    pr: u64,
    head: String,
    publishing: Option<String>,
    checks: Option<String>,
    body: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Opened,
    Updated,
    Ready,
}

struct Event {
    kind: Kind,
    repo: String,
    number: u64,
    head: String,
    publishing: String,
    no_ci: bool,
    body: Option<String>,
}

impl Event {
    fn checked(a: PrArgs) -> Result<Event> {
        let kind = match a.event.as_str() {
            "opened" => Kind::Opened,
            "updated" => Kind::Updated,
            "ready" => Kind::Ready,
            other => bail!("wf.pr: event must be opened, updated or ready, got {other:?}"),
        };
        let repo = a
            .repo
            .trim()
            .split_once('/')
            .filter(|(o, n)| gh_name(o) && gh_name(n))
            .map(|(o, n)| format!("{o}/{n}").to_ascii_lowercase())
            .ok_or_else(|| anyhow!("wf.pr: repo must be \"owner/name\", got {:?}", a.repo))?;
        if a.pr == 0 {
            bail!("wf.pr: pr must be a pull request number");
        }
        let head = a.head.trim().to_ascii_lowercase();
        if !(7..=40).contains(&head.len()) || !head.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("wf.pr: head must be the head commit SHA, 7 to 40 hex characters, got {:?}", a.head);
        }
        let publishing = a.publishing.unwrap_or_default().trim().to_string();
        if kind == Kind::Opened && publishing.is_empty() {
            bail!("wf.pr: opened needs publishing, the Publishing line from the procedure's Publishing section");
        }
        let no_ci = match a.checks.as_deref().map(str::trim) {
            None | Some("pending") => false,
            Some("none") => true,
            Some(other) => bail!("wf.pr: checks must be \"pending\" or \"none\", got {other:?}"),
        };
        if a.checks.is_some() && kind != Kind::Opened {
            bail!("wf.pr: checks applies only to opened");
        }
        let body = a.body.map(|b| b.trim().to_string()).filter(|b| !b.is_empty());
        Ok(Event { kind, repo, number: a.pr, head, publishing, no_ci, body })
    }

    fn of_link(p: &LinkPr) -> Event {
        Event {
            kind: Kind::Ready,
            repo: p.repo.clone(),
            number: p.number,
            head: p.head.clone(),
            publishing: String::new(),
            no_ci: false,
            body: None,
        }
    }

    fn url(&self) -> String {
        format!("https://github.com/{}/pull/{}", self.repo, self.number)
    }

    fn label(&self) -> String {
        format!("{}#{}", self.repo, self.number)
    }

    fn short(&self) -> &str {
        short(&self.head)
    }
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

pub(crate) fn same_head(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && (a.starts_with(b) || b.starts_with(a))
}

pub(crate) fn checked_brief(b: PrBrief) -> Result<PrBrief> {
    let format = b.format.trim().to_string();
    if format.is_empty() && !b.merge_only && b.method.is_none() {
        bail!(
            "wf.openChild: pr.format must name the PR description format, such as \"pr\", or pr.mergeOnly or pr.method must be set"
        );
    }
    let blank = |s: &String| s.trim().is_empty();
    if b.sections.keys().any(blank) || b.sections.values().flatten().any(blank) {
        bail!("wf.openChild: pr.sections maps each heading to the elements it must hold, with no empty names");
    }
    if b.passes.iter().any(blank) {
        bail!("wf.openChild: pr.passes must not hold an empty name");
    }
    let mut sections: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (k, v) in b.sections {
        let entry = sections.entry(k.trim().to_string()).or_default();
        for s in v {
            let item = s.trim().to_string();
            if !entry.contains(&item) {
                entry.push(item);
            }
        }
    }
    let passes = b.passes.into_iter().map(|s| s.trim().to_string()).collect();
    let method = match b.method.map(|m| m.trim().to_ascii_lowercase()) {
        Some(m) if !["merge", "squash", "rebase"].contains(&m.as_str()) => {
            bail!("wf.openChild: pr.method must be merge, squash or rebase, got {m:?}")
        }
        m => m,
    };
    Ok(PrBrief { format, sections, passes, merge_only: b.merge_only, method })
}

fn names(text: &str, name: &str) -> bool {
    let (text, name) = (text.to_ascii_lowercase(), name.trim().to_ascii_lowercase());
    let word = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    text.match_indices(&name).any(|(i, _)| {
        let before = text[..i].chars().next_back().is_none_or(|c| !word(c));
        let after = text[i + name.len()..].chars().next().is_none_or(|c| !word(c));
        before && after
    })
}

fn names_skill(text: &str, name: &str) -> bool {
    names(text, name) || name.rsplit_once(':').is_some_and(|(_, bare)| names(text, bare))
}

fn publishing_gaps(line: &str, brief: &PrBrief, passes: &[String]) -> Vec<String> {
    let mut gaps = Vec::new();
    if !names_skill(line, &brief.format) {
        gaps.push(format!("the {} format on the Publishing line", brief.format));
    }
    let mut seen: Vec<String> = Vec::new();
    for pass in passes.iter().chain(&brief.passes) {
        if seen.iter().any(|s| s.eq_ignore_ascii_case(pass)) {
            continue;
        }
        seen.push(pass.clone());
        if !names_skill(line, pass) {
            gaps.push(format!("the {pass} pass on the Publishing line"));
        }
    }
    gaps
}

fn fence_step(open: &mut Option<String>, line: &str) -> bool {
    let indent = line.bytes().take_while(|&b| b == b' ').count();
    if indent > 3 {
        return open.is_some();
    }
    let t = &line[indent..];
    let Some(c) = t.chars().next().filter(|c| *c == '`' || *c == '~') else { return open.is_some() };
    let run: String = t.chars().take_while(|x| *x == c).collect();
    if run.len() < 3 {
        return open.is_some();
    }
    let rest = &t[run.len()..];
    match open {
        Some(o) if o.starts_with(c) && run.len() >= o.len() && rest.trim().is_empty() => *open = None,
        Some(_) => {}
        None if c == '`' && rest.contains('`') => return false,
        None => *open = Some(run),
    }
    true
}

fn has_fenced_block(lines: &[&str]) -> bool {
    let mut fence = None;
    for line in lines {
        let was_open = fence.is_some();
        fence_step(&mut fence, line);
        if was_open && fence.is_none() {
            return true;
        }
    }
    false
}

fn headings(body: &str) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut fence = None;
    for (i, line) in body.lines().enumerate() {
        let t = line.trim_start();
        if fence_step(&mut fence, line) {
            continue;
        }
        let level = t.chars().take_while(|c| *c == '#').count();
        if level == 0 || level > 6 || !t[level..].starts_with([' ', '\t']) {
            continue;
        }
        out.push((i, level, t[level..].trim().trim_end_matches('#').trim().to_string()));
    }
    out
}

fn required(brief: &PrBrief) -> BTreeMap<String, Vec<String>> {
    let mut want: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if brief.format.eq_ignore_ascii_case("pr") {
        for (heading, needs) in PR_FORMAT {
            want.insert(heading.to_string(), needs.iter().map(|n| n.to_string()).collect());
        }
    }
    for (heading, needs) in &brief.sections {
        let key = want.keys().find(|k| k.eq_ignore_ascii_case(heading)).cloned().unwrap_or_else(|| heading.clone());
        let list = want.entry(key).or_default();
        for n in needs {
            if !list.contains(n) {
                list.push(n.clone());
            }
        }
    }
    want
}

fn description_gaps(body: &str, brief: &PrBrief, repo: &str, refs: &[(String, String)]) -> Vec<String> {
    let lines: Vec<&str> = body.lines().collect();
    let found = headings(body);
    let mut gaps = Vec::new();
    for (heading, needs) in required(brief) {
        let Some(at) = found.iter().position(|(_, _, t)| t.eq_ignore_ascii_case(&heading)) else {
            gaps.push(format!("the {heading} heading"));
            continue;
        };
        let (start, level, _) = &found[at];
        let end = found[at + 1..].iter().find(|(_, l, _)| l <= level).map_or(lines.len(), |(i, _, _)| *i);
        let section_lines = &lines[start + 1..end];
        let section = section_lines.join("\n").to_ascii_lowercase();
        for need in needs {
            let missing = if need == FENCE {
                !has_fenced_block(section_lines)
            } else {
                !section.contains(&need.to_ascii_lowercase())
            };
            if missing {
                let what = if need == FENCE { "a fenced block".to_string() } else { format!("{need:?}") };
                gaps.push(format!("{what} under {heading}"));
            }
        }
    }
    let found = refs_in(body, repo);
    for (slug, shown) in refs {
        if !found.contains(slug) {
            gaps.push(format!("a \"Refs {shown}\" line"));
        }
    }
    gaps
}

fn refs_in(body: &str, repo: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fence = None;
    for line in body.lines() {
        if fence_step(&mut fence, line) {
            continue;
        }
        let mut words = line.split_whitespace();
        if !words.next().is_some_and(|w| w.trim_end_matches(':').eq_ignore_ascii_case("refs")) {
            continue;
        }
        for w in words {
            let w = w.trim_start_matches('(').trim_end_matches([',', '.', ';', ')']);
            let raw = match w.strip_prefix('#') {
                Some(n) => format!("{repo}#{n}"),
                None => w.to_string(),
            };
            if let Some(i) = parse_issue_only(&raw) {
                out.push(i.slug().to_ascii_lowercase());
            }
        }
    }
    out
}

fn issue_refs(ln: &Link, repo: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for s in [&ln.identity.issue, &ln.identity.parent_issue].into_iter().flatten() {
        if let Some(i) = parse_issue_only(s) {
            let slug = i.slug().to_ascii_lowercase();
            if out.iter().any(|(x, _)| x == &slug) {
                continue;
            }
            let same = i.tracker == Tracker::Github && i.scope.eq_ignore_ascii_case(repo);
            let shown = if same { format!("#{}", i.key) } else { i.slug() };
            out.push((slug, shown));
        }
    }
    out
}

fn part(ln: &Link) -> Option<(u32, u32)> {
    let (n, total) = ln.identity.part.as_deref()?.split_once('/')?;
    Some((n.trim().parse().ok()?, total.trim().parse().ok()?))
}

fn superseded() -> Value {
    json!({"status": "superseded", "detail": "a newer head of this PR was recorded while t3cm checked this one"})
}

pub(crate) fn chain_label(ids: &[&str]) -> String {
    match ids {
        [] => String::new(),
        [one] => one.to_string(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

struct Step {
    id: String,
    kind: String,
    gate: String,
    scope: String,
}

impl Step {
    fn of(v: &Value) -> Step {
        let s = |k: &str, d: &str| v[k].as_str().unwrap_or(d).to_string();
        Step { id: s("id", "?"), kind: s("kind", "skill"), gate: s("gate", "always"), scope: s("scope", "pr") }
    }
}

struct Plan {
    options: Vec<String>,
    skipped: Vec<String>,
    later: Vec<String>,
    after: Vec<CardStage>,
}

fn matches_paths(step: &Value, files: &[String]) -> bool {
    let Some(globs) = step["paths"].as_array() else { return true };
    let globs: Vec<&str> = globs.iter().filter_map(Value::as_str).collect();
    files.iter().any(|f| globs.iter().any(|g| glob_matches(g, f).unwrap_or(false)))
}

fn plan(delivery: &Value, files: &[String], combined: Option<&[String]>) -> std::result::Result<Plan, String> {
    if let Some(p) = delivery["problem"].as_str() {
        return Err(format!("the delivery slot has a problem ({p})"));
    }
    let last = combined.is_some();
    let mut active = Vec::new();
    let mut skipped = Vec::new();
    for v in delivery["steps"].as_array().into_iter().flatten() {
        let step = Step::of(v);
        if step.scope == "post-handoff" {
            continue;
        }
        if step.scope == "ticket" && !last {
            active.push(step);
            continue;
        }
        let on = if step.scope == "pr" { files } else { combined.unwrap_or(files) };
        if !matches_paths(v, on) {
            skipped.push(format!("{} (no changed file matches its paths)", step.id));
        } else if step.kind == "skill" && v["skill"]["status"] == "missing" {
            skipped.push(format!("{} (its skill no longer resolves)", step.id));
        } else {
            active.push(step);
        }
    }
    let first = active.first().filter(|s| s.id == "merge" && s.kind == "builtin");
    if first.is_none() {
        return Err("the delivery slot has no builtin merge stage for this PR".into());
    }
    let after: Vec<CardStage> = active[1..]
        .iter()
        .filter(|s| s.scope == "pr" || last)
        .map(|s| CardStage { id: s.id.clone(), scope: s.scope.clone() })
        .collect();
    let mut chain: Vec<&str> = vec!["merge"];
    let mut rest = active[1..].iter();
    let mut later = Vec::new();
    for s in rest.by_ref() {
        let runs_now = s.scope == "pr" || (s.scope == "ticket" && last);
        if s.gate == "chain" && s.kind != "manual" && runs_now {
            chain.push(&s.id);
        } else {
            if s.gate != "never" {
                later.push(s.id.clone());
            }
            break;
        }
    }
    later.extend(rest.filter(|s| s.gate != "never").map(|s| s.id.clone()));
    let mut options: Vec<String> = (1..=chain.len())
        .rev()
        .map(|k| chain_label(&chain[..k]))
        .filter(|o| o.chars().count() <= MAX_OPTION_CHARS)
        .collect();
    if options.len() > MAX_OPTIONS {
        options.drain(..options.len() - MAX_OPTIONS);
    }
    let kept = options.len();
    if kept < chain.len() {
        let mut omitted: Vec<String> = chain[kept..].iter().map(|s| (*s).to_string()).collect();
        omitted.extend(later);
        later = omitted;
    }
    let options = checked_options(&options).map_err(|e| format!("the delivery stages make no valid answers ({e})"))?;
    Ok(Plan { options, skipped, later, after })
}

fn action_line(e: &Event, head: &str, options: &[String], blocker: Option<&str>) -> String {
    let clauses: Vec<String> = options
        .iter()
        .map(|o| match o.as_str() {
            "merge" if options.len() == 1 => "\"merge\" to merge it".to_string(),
            "merge" => "\"merge\" to merge only".to_string(),
            o => {
                let stages = o.strip_prefix("merge and ").or_else(|| o.strip_prefix("merge, ")).unwrap_or(o);
                format!("\"{o}\" to merge and then run {stages}")
            }
        })
        .collect();
    let reply = match clauses.len() {
        n if n > 3 => {
            let quoted: Vec<String> = options.iter().map(|o| format!("\"{o}\"")).collect();
            let (last, init) = quoted.split_last().unwrap_or((&quoted[0], &[]));
            format!("{} or {last} to merge and then run the stages each answer names", init.join(", "))
        }
        _ => clauses.join(", or "),
    };
    let block = blocker
        .map(|b| {
            format!(
                " (GitHub blocks it: {b}; an admin can merge it over the block on GitHub, and the board picks that up)"
            )
        })
        .unwrap_or_default();
    format!(
        "**ACTION NEEDED:** merge #{} at head {}{block}: reply {}, or review it first, here: {}",
        e.number,
        short(head),
        reply,
        e.url()
    )
}

fn change_requests(pr: &Value, head: &str) -> String {
    let reviews = pr["latestOpinionatedReviews"]["nodes"].as_array().into_iter().flatten();
    let named: Vec<String> = reviews
        .filter(|r| r["state"] == "CHANGES_REQUESTED")
        .map(|r| {
            let who = r["author"]["login"].as_str().unwrap_or("a deleted account");
            let at = match r["commit"]["oid"].as_str().filter(|c| !c.is_empty()) {
                Some(c) if same_head(&c.to_ascii_lowercase(), head) => format!("the current head {}", short(head)),
                Some(c) => format!("earlier head {}", short(c)),
                None => "a commit GitHub no longer shows".to_string(),
            };
            format!("by {who} at {at}")
        })
        .collect();
    match named.as_slice() {
        [] => String::new(),
        [one] => format!(", with changes requested {one}"),
        [init @ .., last] => format!(", with changes requested {} and {last}", init.join(", ")),
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ReadySource {
    Direct,
    Reconcile,
}

impl Wf {
    pub(crate) async fn pr_call(&self, caller: Option<&str>, rep: &Reported, args: Value) -> Result<Value> {
        let me = caller.filter(|c| !c.is_empty()).ok_or_else(|| anyhow!(NO_CALLER))?;
        let args = if args.is_null() { json!({}) } else { args };
        let a: PrArgs = serde_json::from_value(args).map_err(|e| anyhow!("wf.pr: {e}"))?;
        let e = Event::checked(a)?;
        let (ln, ticket) = {
            let _g = self.lock.lock().await;
            let (mut ln, ticket) = self.route(me, &e)?;
            let followup = ln.pr.as_ref().is_some_and(|p| p.repo != e.repo || p.number != e.number);
            if e.kind == Kind::Updated
                && let Ok(ticket) = &ticket
                && !ln.pr.as_ref().is_some_and(|p| p.no_ci)
            {
                return self.pr_updated_locked(me, &ln, ticket, &e);
            }
            if e.kind != Kind::Ready || followup {
                ln = self.remember(me, &e, &e.head, false, true)?;
            }
            (ln, ticket)
        };
        let ticket = match ticket {
            Ok(t) => t,
            Err(why) => {
                let verified = !ln.pr.as_ref().is_some_and(|p| p.no_ci);
                return self.to_ticket(me, &ln, &e, &why, None, verified).await;
            }
        };
        match e.kind {
            Kind::Opened => self.pr_opened(me, rep, &ln, &ticket, &e).await,
            Kind::Updated => {
                let why = "this repo runs no CI on PRs, so the ticket runs the brief's checks on the new head";
                self.to_ticket(me, &ln, &e, why, None, true).await
            }
            Kind::Ready if ln.brief.as_ref().is_some_and(|b| b.merge_only) => {
                let why = "the ticket launched this as a targeted fix PR, which takes merge-only delivery";
                self.to_ticket(me, &ln, &e, why, None, true).await
            }
            Kind::Ready if ln.pr.as_ref().is_some_and(|p| p.no_ci) => {
                let why = "this repo runs no CI on PRs, so the ticket runs the brief's checks on the PR head";
                self.to_ticket(me, &ln, &e, why, None, false).await
            }
            Kind::Ready => self.pr_ready(me, rep, &ln, &ticket, &e, ReadySource::Direct).await,
        }
    }

    fn route(&self, me: &str, e: &Event) -> Result<(Link, std::result::Result<Link, String>)> {
        let ln = self.store.link_get(me)?.filter(|l| !l.closed).ok_or_else(|| {
            anyhow!("wf.pr: this thread has no open parent link; tell the user about the PR directly")
        })?;
        let policy = self.config().link_policy(ln.parent_role.as_deref(), &ln.role);
        let ticket = self.store.link_get(&ln.parent)?.filter(|t| !t.closed && t.parent == BOARD);
        let other = ln.pr.as_ref().filter(|p| p.repo != e.repo || p.number != e.number);
        let why = if ln.parent == BOARD {
            Some("this thread reports to the board itself".to_string())
        } else if !policy.relay {
            Some("relay is off for this link in t3cm's roles config".into())
        } else if ticket.is_none() {
            Some("the ticket reports to an overseer thread, not to the board".into())
        } else {
            other.map(|p| format!("this worker already sent {}#{}, so this one is a follow-up PR", p.repo, p.number))
        };
        Ok(match (why, ticket) {
            (None, Some(t)) => (ln, Ok(t)),
            (why, _) => (ln, Err(why.unwrap_or_default())),
        })
    }

    fn touch(&self, ln: &mut Link) {
        ln.last_up = self.now();
        ln.nudges = 0;
        ln.escalated = false;
    }

    fn remember(&self, me: &str, e: &Event, head: &str, waiting: bool, touch_up: bool) -> Result<Link> {
        let mut ln = self.store.link_get(me)?.ok_or_else(|| anyhow!("wf.pr: this thread has no parent link"))?;
        match ln.pr.as_mut() {
            Some(p) if p.repo == e.repo && p.number == e.number => {
                if e.kind == Kind::Ready || !same_head(&p.head, head) {
                    p.head = head.into();
                    p.waiting = waiting;
                } else if head.len() > p.head.len() {
                    p.head = head.into();
                }
                if e.kind == Kind::Opened {
                    p.no_ci = e.no_ci;
                }
            }
            Some(p) => {
                let other = (e.repo.clone(), e.number);
                if !p.followups.contains(&other) {
                    p.followups.push(other);
                }
            }
            None => {
                let (repo, number, no_ci) = (e.repo.clone(), e.number, e.no_ci);
                ln.pr = Some(LinkPr { repo, number, head: head.into(), waiting, no_ci, followups: Vec::new() });
            }
        }
        if touch_up {
            self.touch(&mut ln);
        }
        self.store.link_save(&ln)?;
        Ok(ln)
    }

    async fn fallback(
        &self,
        me: &str,
        ln: &Link,
        e: &Event,
        why: &str,
        extra: Option<&str>,
        checks_verified: bool,
    ) -> Result<Value> {
        self.send_to_ticket(me, ln, e, why, extra, Some(ln.pr.clone()), checks_verified).await
    }

    fn moved_on(&self, me: &str, seen: &Link) -> Result<bool> {
        Ok(self.store.link_get(me)?.is_none_or(|l| l.closed || l.parent != seen.parent || l.pr != seen.pr))
    }

    pub(crate) fn handled(&self, worker: &str, ticket: &str, note: &str) -> Result<String> {
        let h = self.store.msg_new(WATCHER, ticket, HANDLED, note, &self.now())?;
        if let Some(mut wl) = self.store.link_get(worker)? {
            wl.unread.push(h.clone());
            self.store.link_save(&wl)?;
        }
        Ok(h)
    }

    fn event_text(ln: &Link, e: &Event, checks_verified: bool) -> (&'static str, Option<String>, String) {
        let part = ln.identity.part.as_deref().map(|p| format!("PR {p}")).unwrap_or_else(|| "PR".into());
        let (n, url, head) = (e.number, e.url(), &e.head);
        let (kind, gate, mut body) = match e.kind {
            Kind::Opened => {
                let checks = if e.no_ci { "none in this repo" } else { "pending" };
                (
                    "event",
                    None,
                    format!("{part} opened: #{n} {url}, head {head}\nPublishing: {}\nChecks: {checks}", e.publishing),
                )
            }
            Kind::Updated => ("event", None, format!("{part} updated: #{n}, head {head}")),
            Kind::Ready => {
                let suffix = if checks_verified { ", checks passed" } else { "" };
                ("gate", Some("merge".into()), format!("{part} ready to merge: #{n}, head {head}{suffix}, here: {url}"))
            }
        };
        if let Some(b) = &e.body {
            body.push('\n');
            body.push_str(b);
        }
        (kind, gate, body)
    }

    async fn to_ticket(
        &self,
        me: &str,
        ln: &Link,
        e: &Event,
        why: &str,
        extra: Option<&str>,
        checks_verified: bool,
    ) -> Result<Value> {
        self.send_to_ticket(me, ln, e, why, extra, None, checks_verified).await
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_to_ticket(
        &self,
        me: &str,
        ln: &Link,
        e: &Event,
        why: &str,
        extra: Option<&str>,
        expect_pr: Option<Option<LinkPr>>,
        checks_verified: bool,
    ) -> Result<Value> {
        let (kind, gate, mut body) = Self::event_text(ln, e, checks_verified);
        if let Some(x) = extra {
            body.push('\n');
            body.push_str(x);
        }
        body.push_str(&format!("\n\nt3cm left this to the ticket: {why}."));
        let args = SendArgs { kind: kind.into(), body, gate, wake: None, options: None, expect_pr };
        let mut out = self.send(me, args).await?;
        if out["status"] == "superseded" {
            return Ok(out);
        }
        out["status"] = json!("ticket");
        out["why"] = json!(why);
        Ok(out)
    }

    async fn slots_of(&self, me: &str, rep: &Reported) -> Result<Value> {
        let ctx = self.slot_ctx(Some(me), rep, None, None).await?;
        let r = self.resolve_slots(&ctx, &[]).await?;
        Ok(r.all(&Caps::Unchecked))
    }

    async fn read_pr(&self, repo: &str, number: u64) -> Result<Value> {
        let (owner, name) = repo.split_once('/').unwrap_or_default();
        let query = format!("{PR_READ} {CHECKS}");
        let r = self.github.graphql(&query, json!({"owner": owner, "name": name, "number": number})).await?;
        if let Some(e) = errors(&r) {
            bail!("{e}");
        }
        let pr = r["data"]["repository"]["pullRequest"].clone();
        if pr.is_null() {
            bail!("{repo}#{number} is not a pull request");
        }
        Ok(pr)
    }

    async fn pr_opened(&self, me: &str, rep: &Reported, ln: &Link, _ticket: &Link, e: &Event) -> Result<Value> {
        let Some(brief) = ln.brief.clone().filter(|b| !b.format.is_empty()) else {
            let why = "the ticket's wf.openChild gave no PR format to check the description against";
            return self.to_ticket(me, ln, e, why, None, true).await;
        };
        let passes: Vec<String> = match self.slots_of(me, rep).await {
            Ok(eff) => eff["passes"]["skills"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|s| s["ref"].as_str().or(s["name"].as_str()).map(String::from))
                .collect(),
            Err(err) => {
                let why = format!("t3cm could not resolve the passes slot ({err:#})");
                return self.to_ticket(me, ln, e, &why, None, true).await;
            }
        };
        let pr = match self.read_pr(&e.repo, e.number).await {
            Ok(pr) => pr,
            Err(err) => {
                let why = format!("t3cm could not read {} on GitHub ({err:#})", e.label());
                return self.to_ticket(me, ln, e, &why, None, true).await;
            }
        };
        let mut gaps = publishing_gaps(&e.publishing, &brief, &passes);
        let body = pr["body"].as_str().unwrap_or_default();
        gaps.extend(description_gaps(body, &brief, &e.repo, &issue_refs(ln, &e.repo)));
        if gaps.is_empty() && e.no_ci {
            let why = "this repo runs no CI on PRs, so the ticket runs the brief's checks on the PR head";
            let extra = "t3cm checked the description and the Publishing line, and both hold.";
            return self.to_ticket(me, ln, e, why, Some(extra), true).await;
        }
        let _g = self.lock.lock().await;
        let Some(wl) = self.store.link_get(me)?.filter(|l| !l.closed && l.pr == ln.pr) else {
            return Ok(superseded());
        };
        let Some(cur_ticket) = self.store.link_get(&wl.parent)?.filter(|t| !t.closed && t.parent == BOARD) else {
            return Ok(superseded());
        };
        let (_, _, text) = Self::event_text(&wl, e, true);
        let h = self.store.msg_new(me, &cur_ticket.child, "event", &text, &self.now())?;
        if !gaps.is_empty() {
            let missing = gaps.join("; ");
            let message = format!(
                "Rewrite #{}'s description through the Publishing section of the t3-worker procedure: {missing}.",
                e.number
            );
            let note = format!(
                "{} opened {} ({h}); t3cm sent the worker the rewrite request, as it is missing {missing}",
                wl.label,
                e.url()
            );
            self.handled(me, &cur_ticket.child, &note)?;
            return Ok(json!({"status": "rewrite", "handle": h, "message": message, "missing": gaps,
                "detail": "do what message says, then send wf.pr opened again for this PR"}));
        }
        let note = format!(
            "{} opened {} at head {} ({h}); t3cm checked its description and Publishing line",
            wl.label,
            e.url(),
            e.short()
        );
        self.handled(me, &cur_ticket.child, &note)?;
        Ok(json!({"status": "open", "handle": h,
            "detail": "t3cm checked the description and the Publishing line; send wf.pr ready once the checks pass"}))
    }

    fn pr_updated_locked(&self, me: &str, prev: &Link, ticket: &Link, e: &Event) -> Result<Value> {
        let moved = prev.pr.as_ref().is_none_or(|p| !same_head(&p.head, &e.head));
        let mut wl = self.remember(me, e, &e.head, false, true)?;
        let (_, _, text) = Self::event_text(&wl, e, true);
        let h = self.store.msg_new(me, &ticket.child, "event", &text, &self.now())?;
        let stale = |c: &Card| {
            c.kind == "gate"
                && c.worker.is_none()
                && c.ticket.as_deref() == Some(ticket.child.as_str())
                && c.pr.as_ref().is_some_and(|p| {
                    p.repo == e.repo && p.number == e.number && p.head.as_ref().is_some_and(|h| !same_head(h, &e.head))
                })
        };
        let old: Vec<Card> = self.cards()?.into_iter().filter(stale).collect();
        let ids: Vec<String> = old.iter().map(|c| c.id.clone()).collect();
        if !old.is_empty() {
            let seqs: Vec<i64> = old.iter().map(|c| c.seq).collect();
            let note = format!("superseded: the PR moved to head {}", e.short());
            self.store.waiting_resolve_releasing(BOARD, &seqs, &[ticket.child.as_str()], &note, &self.now())?;
            self.sync_merge_gate_pr(&ticket.child)?;
            self.sync_relay_gate(&ticket.child)?;
            self.changed();
        }
        if moved && wl.gate.as_deref() == Some("merge") {
            wl.gate = None;
            wl.gate_since = None;
            self.store.link_save(&wl)?;
        }
        let what = e.body.as_deref().map(|b| format!(": {}", crate::t3::truncate_chars(b, 120))).unwrap_or_default();
        let dropped = if ids.is_empty() {
            String::new()
        } else {
            format!("; t3cm dropped merge card {} for the old head", ids.join(", "))
        };
        let note = format!("{} moved {} to head {} ({h}){what}{dropped}", wl.label, e.url(), e.short());
        self.handled(me, &ticket.child, &note)?;
        Ok(json!({"status": "recorded", "handle": h, "dropped": ids,
            "detail": "t3cm recorded the new head; send wf.pr ready for it once its checks pass"}))
    }

    pub(crate) fn queue_worker_reply(&self, worker: &str, body: &str) -> Result<()> {
        let h = self.store.msg_new(WATCHER, worker, "reply", body, &self.now())?;
        let text = format!("{}\n{body}", header("reply", WATCHER, &h, None));
        self.store.outbox_push(worker, &text, &h, &self.now())
    }

    async fn refuse_ready(&self, me: &str, ln: &Link, e: &Event, source: ReadySource, reason: String) -> Result<Value> {
        let _g = self.lock.lock().await;
        if source == ReadySource::Direct {
            if let Some(mut wl) = self.store.link_get(me)?.filter(|l| !l.closed) {
                self.touch(&mut wl);
                self.store.link_save(&wl)?;
            }
            return Ok(json!({"status": "refused", "reason": reason}));
        }
        if self.moved_on(me, ln)? {
            return Ok(superseded());
        }
        let body = format!("t3cm could not open the merge gate for {}: {reason}.", e.url());
        self.queue_worker_reply(me, &body)?;
        if let Some(mut wl) = self.store.link_get(me)?
            && let Some(lp) = wl.pr.as_mut()
        {
            lp.waiting = false;
            self.store.link_save(&wl)?;
        }
        Ok(json!({"status": "refused", "reason": reason, "told": true}))
    }

    async fn pr_ready(
        &self,
        me: &str,
        rep: &Reported,
        ln: &Link,
        ticket: &Link,
        e: &Event,
        source: ReadySource,
    ) -> Result<Value> {
        let pr = match self.read_pr(&e.repo, e.number).await {
            Ok(pr) => pr,
            Err(err) => {
                let why = format!("t3cm could not read {} on GitHub ({err:#})", e.label());
                return self.fallback(me, ln, e, &why, None, false).await;
            }
        };
        let head = pr["headRefOid"].as_str().unwrap_or_default().to_ascii_lowercase();
        if pr["state"] != "OPEN" {
            let state = pr["state"].as_str().unwrap_or("unknown").to_ascii_lowercase();
            return self.refuse_ready(me, ln, e, source, format!("{} is {state}", e.label())).await;
        }
        if pr["isDraft"] == true {
            let reason = format!("{} is a draft; mark it ready for review, then send ready again", e.label());
            return self.refuse_ready(me, ln, e, source, reason).await;
        }
        if !same_head(&head, &e.head) {
            let reason = format!(
                "the head moved: GitHub shows {} at {head}; send wf.pr updated for it, then ready once its checks pass",
                e.label()
            );
            return self.refuse_ready(me, ln, e, source, reason).await;
        }
        let state = pr["mergeStateStatus"].as_str().unwrap_or("UNKNOWN");
        let checks = check_state(&pr);
        let pending = match (&checks, state) {
            (CheckState::Pending(p), _) => Some(format!("checks still running: {p}")),
            (CheckState::Passed(_), "UNKNOWN") => Some("GitHub is still computing the merge state".into()),
            _ => None,
        };
        let non_success = match checks {
            CheckState::Failing(f) => {
                let _g = self.lock.lock().await;
                if self.moved_on(me, ln)? {
                    return Ok(superseded());
                }
                if source == ReadySource::Reconcile {
                    let body = format!(
                        "A check failed on {} at head {}: {f}. Fix it and push, then send wf.pr updated, and ready once the checks pass.",
                        e.url(),
                        short(&head)
                    );
                    self.queue_worker_reply(me, &body)?;
                }
                let note = format!(
                    "{} sent ready for {} at head {}, but a check failed: {f}; t3cm sent it back to the worker",
                    ln.label,
                    e.url(),
                    short(&head)
                );
                self.handled(me, &ticket.child, &note)?;
                self.remember(me, e, &head, false, source == ReadySource::Direct)?;
                let mut out = json!({"status": "failing", "checks": f,
                    "detail": "fix the failing checks and push, then send wf.pr updated, and ready once they pass"});
                if source == ReadySource::Reconcile {
                    out["told"] = json!(true);
                }
                return Ok(out);
            }
            CheckState::Unread => {
                let why = format!(
                    "GitHub shows no finished check run at head {}, so t3cm cannot verify the checks",
                    short(&head)
                );
                return self.fallback(me, ln, e, &why, None, false).await;
            }
            CheckState::Passed(settled) => settled,
            CheckState::Pending(_) => Vec::new(),
        };
        if let Some(wait) = pending {
            if source == ReadySource::Direct && rep != &Reported::default() {
                let _ = self.slot_ctx(Some(me), rep, None, None).await;
            }
            let _g = self.lock.lock().await;
            if self.moved_on(me, ln)? {
                return Ok(superseded());
            }
            let was_waiting =
                self.store.link_get(me)?.and_then(|l| l.pr).is_some_and(|p| p.waiting && same_head(&p.head, &head));
            self.remember(me, e, &head, true, source == ReadySource::Direct)?;
            if !was_waiting {
                let note = format!(
                    "{} sent ready for {} at head {}; t3cm is waiting ({wait})",
                    ln.label,
                    e.url(),
                    short(&head)
                );
                self.handled(me, &ticket.child, &note)?;
            }
            let reconcile = self.config().board.reconcile_idle_minutes > 0;
            let detail = if reconcile {
                "t3cm opens the merge gate once they finish; a failed check comes back to you as a reply"
            } else {
                "GitHub reconcile is off, so send wf.pr ready again once they finish"
            };
            return Ok(json!({"status": "waiting", "pending": wait, "detail": detail}));
        }
        if state == "DIRTY" {
            let reason = format!(
                "{} has a merge conflict; rebase it on the default branch, then send updated and ready",
                e.label()
            );
            return self.refuse_ready(me, ln, e, source, reason).await;
        }
        let blocker = matches!(state, "BLOCKED" | "BEHIND").then(|| {
            let review = pr["reviewDecision"].as_str().map(|r| format!(", review decision {r}")).unwrap_or_default();
            format!("merge state {state}{review}{}", change_requests(&pr, &head))
        });
        let nodes = pr["files"]["nodes"].as_array().cloned().unwrap_or_default();
        let files: Vec<String> = nodes.iter().filter_map(|f| f["path"].as_str().map(String::from)).collect();
        if pr["files"]["pageInfo"]["hasNextPage"] == true {
            return self
                .fallback(me, ln, e, "the PR changes more than 100 files, past what t3cm reads", None, true)
                .await;
        }
        let Some((n, total)) = part(ln) else {
            let why = "the worker's link has no part n/N, so t3cm cannot tell whether this is the ticket's last PR";
            return self.fallback(me, ln, e, why, None, true).await;
        };
        let delivery = match self.slots_of(me, rep).await {
            Ok(eff) => eff["delivery"].clone(),
            Err(err) => {
                let why = format!("t3cm could not resolve the delivery slot ({err:#})");
                return self.fallback(me, ln, e, &why, None, true).await;
            }
        };
        let combined = match n >= total {
            false => None,
            true => match self.ticket_files(me, ticket, e, &files, &delivery).await {
                Ok(c) => Some(c),
                Err(why) => return self.fallback(me, ln, e, &why, None, true).await,
            },
        };
        let plan = match plan(&delivery, &files, combined.as_deref()) {
            Ok(p) => p,
            Err(why) => return self.fallback(me, ln, e, &why, None, true).await,
        };
        let mut body = action_line(e, &head, &plan.options, blocker.as_deref());
        let checks_summary = if non_success.is_empty() {
            format!("Every check passed at head {head}.")
        } else {
            format!("Every check met merge policy at head {head} (settled non-success: {}).", non_success.join(", "))
        };
        body.push_str(&format!("\n\n{checks_summary} A merge approval holds for this head only."));
        if !plan.later.is_empty() {
            body.push_str(&format!("\nLater stages open their own gate: {}.", plan.later.join(", ")));
        }
        if !plan.skipped.is_empty() {
            body.push_str(&format!("\nSkipped for this PR: {}.", plan.skipped.join(", ")));
        }
        let _g = self.lock.lock().await;
        if self.moved_on(me, ln)? {
            return Ok(superseded());
        }
        let Some(mut t) = self.store.link_get(&ticket.child)?.filter(|t| !t.closed) else {
            bail!("the ticket's link closed while t3cm checked the PR");
        };
        let h = self.store.msg_new(me, &t.child, "gate", &body, &self.now())?;
        self.resolve_cards(
            |c| {
                c.kind == "gate"
                    && c.worker.is_none()
                    && c.ticket.as_deref() == Some(t.child.as_str())
                    && c.pr.as_ref().is_some_and(|p| p.repo == e.repo && p.number == e.number)
            },
            "superseded by a newer merge gate",
        )?;
        let card = self.card_add(NewCard {
            kind: "gate",
            label: Some(&t.label),
            ticket: Some(&t.child),
            link: t.link.as_deref(),
            body: &body,
            handle: Some(&h),
            options: Some(plan.options.clone()),
            pr: Some(CardPr {
                repo: e.repo.clone(),
                number: e.number,
                head: Some(head.clone()),
                ready: true,
                after: Some(plan.after.clone()),
            }),
            approval: None,
            offer: None,
            worker: None,
        })?;
        if t.gate.as_deref().is_none_or(|g| g == "merge" || g.starts_with("relay:")) {
            let already_merge = t.gate.as_deref() == Some("merge");
            t.gate = Some("merge".into());
            t.gate_since = Some(self.now());
            if !already_merge {
                t.gate_pr = Some((e.repo.clone(), e.number));
            }
            t.nudges = 0;
            t.escalated = false;
            self.store.link_save(&t)?;
        }
        let mut wl = self.remember(me, e, &head, false, source == ReadySource::Direct)?;
        if wl.gate.is_none() {
            wl.gate = Some("merge".into());
            wl.gate_since = Some(self.now());
            self.store.link_save(&wl)?;
        }
        let quoted: Vec<String> = plan.options.iter().map(|o| json!(o).to_string()).collect();
        let mut note = format!(
            "{} sent ready for {} ({h}); t3cm checked it on GitHub at head {} and opened merge card {card} with {}",
            ln.label,
            e.url(),
            short(&head),
            quoted.join(", ")
        );
        if let Some(b) = &blocker {
            note.push_str(&format!("; GitHub blocks it: {b}"));
        }
        if !plan.skipped.is_empty() {
            note.push_str(&format!("; skipped {}", plan.skipped.join(", ")));
        }
        self.handled(me, &t.child, &note)?;
        self.changed();
        Ok(json!({"status": "gated", "card": card, "options": plan.options, "head": head,
            "detail": format!("on the user's board as card {card}; \"Merged ... Hand off.\" arrives once it merges")}))
    }

    async fn ticket_files(
        &self,
        me: &str,
        ticket: &Link,
        current: &Event,
        files: &[String],
        delivery: &Value,
    ) -> std::result::Result<Vec<String>, String> {
        let needs =
            delivery["steps"].as_array().into_iter().flatten().any(|s| s["scope"] == "ticket" && s["paths"].is_array());
        let mut all: Vec<String> = files.to_vec();
        if !needs {
            return Ok(all);
        }
        let siblings = self.store.children(&ticket.child, false).map_err(|e| format!("{e:#}"))?;
        let mut prs: Vec<(String, u64)> = Vec::new();
        let mut push_pr = |repo: String, number: u64| {
            let key = (repo, number);
            if (key.0 != current.repo || key.1 != current.number) && !prs.contains(&key) {
                prs.push(key);
            }
        };
        for s in siblings.into_iter().filter(|s| s.role == "worker") {
            let Some(p) = &s.pr else {
                if s.child != me && s.closed_how.as_deref() == Some("handoff") {
                    return Err(format!(
                        "{} handed off without sending its PR through wf.pr, so t3cm cannot gather the ticket's changed files",
                        s.label
                    ));
                }
                continue;
            };
            if s.child != me {
                push_pr(p.repo.clone(), p.number);
            }
            for (repo, number) in &p.followups {
                push_pr(repo.clone(), *number);
            }
        }
        for (repo, number) in prs {
            let pr = self
                .read_pr(&repo, number)
                .await
                .map_err(|err| format!("t3cm could not read {repo}#{number} on GitHub ({err:#})"))?;
            if pr["state"] == "CLOSED" {
                continue;
            }
            if pr["files"]["pageInfo"]["hasNextPage"] == true {
                return Err(format!("{repo}#{number} changes more than 100 files, past what t3cm reads"));
            }
            for f in pr["files"]["nodes"].as_array().into_iter().flatten().filter_map(|f| f["path"].as_str()) {
                if !all.iter().any(|x| x == f) {
                    all.push(f.to_string());
                }
            }
        }
        Ok(all)
    }

    pub(crate) fn release_ready_worker(
        &self,
        ticket: &str,
        repo: &str,
        number: u64,
        head: &str,
    ) -> Result<Vec<String>> {
        let mut told = Vec::new();
        for mut wl in self.store.children(ticket, true)? {
            let ours =
                wl.pr.as_ref().is_some_and(|p| p.repo == repo && p.number == number && !same_head(head, &p.head));
            if wl.role != "worker" || !ours || wl.gate.as_deref() != Some("merge") {
                continue;
            }
            wl.gate = None;
            wl.gate_since = None;
            self.store.link_save(&wl)?;
            let body = format!(
                "The head of https://github.com/{repo}/pull/{number} moved to {} outside this thread, so t3cm dropped its merge card. Send wf.pr updated for the new head, and ready once its checks pass.",
                short(head)
            );
            self.queue_worker_reply(&wl.child, &body)?;
            told.push(wl.child);
        }
        Ok(told)
    }

    pub(crate) fn ready_waiting(&self) -> Result<Vec<(Link, LinkPr)>> {
        Ok(self
            .store
            .links_open()?
            .into_iter()
            .filter_map(|l| l.pr.clone().filter(|p| p.waiting).map(|p| (l, p)))
            .collect())
    }

    pub(crate) async fn retry_ready(&self, worker: &Link, p: &LinkPr) -> Result<(String, bool)> {
        let e = Event::of_link(p);
        let (ln, ticket) = {
            let _g = self.lock.lock().await;
            self.route(&worker.child, &e)?
        };
        if ln.pr.as_ref() != Some(p) {
            return Ok((format!("{} ready recheck: superseded", e.label()), false));
        }
        let out = match ticket {
            Ok(t) => self.pr_ready(&worker.child, &Reported::default(), &ln, &t, &e, ReadySource::Reconcile).await?,
            Err(why) => self.fallback(&worker.child, &ln, &e, &why, None, true).await?,
        };
        let status = out["status"].as_str().unwrap_or_default().to_string();
        let told = out["told"] == true;
        Ok((format!("{} ready recheck: {status}", e.label()), told))
    }
}
