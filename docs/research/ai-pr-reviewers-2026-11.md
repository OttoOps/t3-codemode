# AI pull request reviewers in community open source, November 2026

Survey for [#16](https://github.com/potto007/t3-codemode/issues/16). It asks how
45 of the busiest community open source repositories on GitHub run AI pull
request reviewers, and uses the answers to pick a setup for t3-codemode. The
per-repository findings, with every citation, are in
[`ai-pr-reviewers-2026-11.json`](ai-pr-reviewers-2026-11.json) and, one row per
finding, [`ai-pr-reviewers-2026-11.csv`](ai-pr-reviewers-2026-11.csv). The
follow-up to build the recommended reviewer is
[#27](https://github.com/potto007/t3-codemode/issues/27).

## Summary

- **Most of these projects see AI review, and fewer than half run it.** In 36
  of 45 repositories an AI reviewer acted on at least one of the pull requests
  scanned (the last 100 merged since 2026-08-20 and the last 50 open). Only 19
  of 45 run one as a project: 12 run it without anyone asking on the pull
  requests it covers, and 7 ship a workflow or configuration that a maintainer
  or contributor invokes. In 17, AI reviews appear with nothing in the
  repository configuring them, and where the timeline shows who asked, it was
  the pull request's author. 9 show none.
- **GitHub Copilot code review is the default.** It appears in 35 of 45
  repositories, mostly as a contributor's own request with nothing in the
  repository configuring it. OpenAI Codex appears in 6, CodeRabbit in 6,
  Claude in 5.
- **Hosted apps outnumber workflows.** Of the 19 project-run reviewers, 12 are
  hosted apps or services that keep the model credentials off GitHub Actions,
  5 run a review agent inside a GitHub Actions job of the repository, and 2
  (OpenWrt and Envoy) are workflows that start a review that runs elsewhere.
  Gutenberg also runs a model in Actions, to pick human reviewers; it posts no
  review.
- **The bot comments and a human decides.** Apart from Apache Doris and pnpm,
  whose bots also give verdicts (some later dismissed), and one dismissed
  Korbit review on Superset, every AI review observed was a plain comment. In no repository
  whose rules are readable does an AI review count toward a required
  approval.
- **Most publish an AI policy.** 38 of 45 do. Several
  restrict AI review itself: Rust allows review bots only on a separate,
  blockable account and only as advice, NumPy says every reviewer interaction
  must be by a human, and Servo asks contributors not to use AI review tools.
- **systemd's workflow is the most complete self-hosted design.** It adds
  its label only to pull requests whose author is an owner, member or
  collaborator, and accepts `@claude review` only from comment or review
  authors with one of those associations (a label added by hand relies on
  GitHub's triage permission), runs the model
  in a job with a read-only token and an OIDC credential, posts from a
  separate job, pins every action by commit SHA, and caps turns and time. The
  other four repositories that run a review agent in Actions each do some
  of this.

**Recommendation for t3-codemode.** An advisory reviewer that a maintainer
starts with a label, run by a workflow in this repository that splits the work
into three jobs so the model never holds a write token, posting comments only.
The [recommendation](#recommendation-for-t3-codemode) section gives the
design and the reasons.

## Method

**Population and ranking.** On 2026-11-18 a script built a pool of 1,807
repositories from two GitHub searches:

- the top 1,000 results of
  `stars:>=5000 pushed:>=2026-11-01 archived:false fork:false sort:stars`;
- for each of 89 foundation or community GitHub organizations,
  `org:<org> stars:>=300 pushed:>=2026-11-01 archived:false fork:false sort:stars`.

Eligible repositories belong to an owner on that community list (the CNCF and
Apache projects, the Python, Rust, OpenJS, NumFOCUS, PyTorch and Linux
Foundation projects, and community-governed projects such as systemd and
Neovim). I excluded repositories owned by a company or an individual, and
list, tutorial and website repositories. The list and the reason for each
exclusion are in the data file under `survey`.

For each repository with 50 or more open pull requests (835), the script
counted pull requests merged in the window with
`repo:<r> is:pr is:merged merged:>=2026-08-20`. For eligible repositories with
200 or more merges (the top two per owner, six for `apache`), it counted
distinct human authors of those merged pull requests. The score is the mean of
the three percentile ranks (merged pull requests, distinct authors, open pull
requests). Taking repositories in score order, at most one per owner (four for
`apache`), gave the 45. The full ranked list of 105 candidates is in the data
file under `candidates`.

The sample spans 23 foundations or governance homes (9 CNCF, 4 Apache, 4
NumFOCUS, 7 community-governed, and one or two each from the others) and 14
primary languages (10 Python, 8 Go, 5 C, 5 Rust, 4 C++, 3 TypeScript, 3 Ruby,
and others).

**Evidence.** For each repository, a script pinned the default branch head
SHA and fetched every workflow file, any AI review configuration it could find
(`.coderabbit.yaml`, `.gemini/`, `.github/copilot-instructions.md`,
`.pr_agent.toml`, `AGENTS.md` and similar), the contributing guide and pull
request template, and the reviewers and commenters on the last 100 merged and
last 50 open pull requests. Research agents (Claude Sonnet 5.5, 16 batches of
one to four repositories, plus the reference case) read that evidence and
looked up the rest with read-only `gh` calls. Each recorded its findings in a
fixed schema: reviewer, triggers, who can trigger, event, checkout, token
scopes, SHA pinning, secrets, powers, merge gate, configuration, cost controls,
human role, incidents and activity.

Each of the 625 findings carries at least one citation: a file at a pinned
commit with a line range, a pull request where the bot acted, a repository
ruleset, or the project's published policy. A finding that a reviewer is not
configured in the repository cites the root and `.github` directory listings
at the pinned commit, and means only that nothing was found in `.github/workflows` or the AI
configuration paths listed under `survey.config_paths_checked`. A hardening
change cites the commit that made it.
Findings the agents could not cite went to a `dropped` list (109 across the
45 repositories) instead of into the data. Where a worker noted a cited AI
review without recording it as a reviewer (CPython, Cilium, NumPy, Symfony),
I added the row from that citation, marked `added_by`. A script then resolved
all 455 distinct citation URLs in the final data (442 for the 45
repositories, the rest for the reference case) against GitHub, and checked
each line range against the file at its commit. All resolved. I checked a
sample of findings against the workflow files myself and assigned each repository's adoption tier
from the agents' records; the tier and the reason are in the data file.

**Limits.**

- Activity counts come from the last 100 merged and 50 open pull requests, so
  rare reviewers can be missed.
- Branch protection is not readable without admin rights, so whether a check
  is required is often `unknown`. Rulesets are readable, and the data cites
  them where they settle it.
- A hosted app's trigger rule usually lives outside the repository. In those
  cases the data shows what the app did, and says the trigger is unknown.
- Only published security advisories count.

## Adoption

| Tier | Repos | Who |
|---|---:|---|
| Automatic: runs without anyone asking on the pull requests it covers | 12 | vLLM, Home Assistant, Superset, OpenWrt, Kueue, Argo CD, systemd, pnpm, Envoy, Backstage, Valkey, KubeVirt |
| On demand: the project ships it, someone invokes it | 7 | LLVM, SGLang, Zephyr, Airflow, Gutenberg, Doris, ExecuTorch |
| Contributor opt-in only: no project setup | 17 | nixpkgs, CPython, Kubernetes, conda-forge, DataFusion, Node.js, OpenTelemetry Collector Contrib, Rails, Ruby, FreeCAD, Cilium, pandas, Symfony, Prometheus, Gitea, NumPy, DefinitelyTyped |
| None found | 9 | Rust, Godot, DuckDB, Homebrew, Julia, Bevy, PHP, Servo, Neovim |

The opt-in tier is mostly GitHub Copilot code review. A contributor with a
Copilot plan can request it on their own pull request, and in Node.js, Rails
and conda-forge the timeline shows the author doing that. Several of
those reviews contain only a quota notice and no review, for example
[conda-forge #35550](https://github.com/conda-forge/staged-recipes/pull/35550).

## Comparison of the 19 project-run reviewers

"Hosted" means the model runs on the vendor's side and the repository holds no
model credential. Powers are what the bot was seen to do or what its
configuration allows. Every row links its primary source; the full citations
are in the data.

| Repo | Reviewer | Kind | Trigger | Who can trigger | Credential | Pinned | Powers |
|---|---|---|---|---|---|---|---|
| [vLLM](https://github.com/vllm-project/vllm/pull/67106) | Claude (app) | hosted | every same-repo PR; fork PRs on a maintainer's `@claude review` | maintainers for forks | none in repo | n/a | comment |
| [Home Assistant](https://github.com/home-assistant/core/blob/edf0221853d2ea589681fd84e6ba5cf5fc44e093/.github/workflows/quality-scale-reviewer.lock.yml#L96-L99) | Copilot review, plus two gh-aw agentic workflows | hosted + workflow | every PR; workflows on integration PRs | anyone; fork PRs excluded from the workflows | `copilot-requests` permission | yes | comment |
| [Superset](https://github.com/apache/superset/pull/46219) | CodeAnt (also Bito, Copilot) | hosted | about half of PRs | anyone | none in repo | n/a | comment |
| [OpenWrt](https://github.com/openwrt/openwrt/blob/27663a017ad8ac6883e11a81015db0c0fc341a9e/.github/workflows/llm-review.yml#L1-L78) | Claude Code routine | workflow starts a hosted routine | schedule, twice a day, up to 24 open PRs | nobody per PR | routine bearer token | no | comment, suggest |
| [Kueue](https://github.com/kubernetes-sigs/kueue/blob/0523a31c8f1e85bb4bfccf0e871a680b472d5127/.coderabbit.yaml#L1-L134) | CodeRabbit | hosted | every PR; skips drafts, WIP and untrusted-author labels | anyone | none in repo | n/a | comment |
| [Argo CD](https://github.com/argoproj/argo-cd/pull/31046) | CodeRabbit (also Copilot, Qodo) | hosted | nearly every PR | anyone | none in repo | n/a | comment |
| [systemd](https://github.com/systemd/systemd/blob/d26f3cbfbfb52882012803a15be869b112d55539/.github/workflows/claude-review.yml#L26-L69) | Claude Code CLI | workflow | `claude-review` label, auto-added to PRs from owners, members and collaborators, then every push while it stays; `@claude review` | owner, member or collaborator (PR author for the auto-label, comment or review author for mentions); triage rights for a manual label | AWS Bedrock by OIDC | yes | comment, resolve own threads |
| [pnpm](https://github.com/pnpm/pnpm/blob/b0a7dac0fe88264cdef0f258cdb755d5d28c4a8f/.coderabbit.yaml#L1-L27) | CodeRabbit, Qodo, Greptile, Copilot | hosted | every PR | anyone | none in repo | n/a | approve, request changes, label |
| [Envoy](https://github.com/envoyproxy/envoy/blob/fdd02e6b8cbc7fe7e0b6c628e87623a8cb09da50/.github/workflows/copilot-review.yml#L1-L42) | Copilot review, requested by a workflow | workflow requests hosted review | PRs touching `api/**`; Gemini on `/gemini` | anyone touching `api/**` | none in repo | no actions used | comment |
| [Backstage](https://github.com/backstage/backstage/rules/11770548) | Copilot review via ruleset | hosted | every push, not drafts | anyone | none in repo | n/a | comment |
| [Valkey](https://github.com/valkey-io/valkey-review-bot/blob/9780d6bd26aed9e9366bbaedb2a79c529672a1ab/README.md#L1-L55) | CodeRabbit, and the project's own valkey-review-bot | hosted | every PR; own bot debounced, daily budget | anyone; mention needs write access | none in repo | n/a | comment |
| [KubeVirt](https://github.com/kubevirt/kubevirt/blob/79a4116d3951b0369203ba00d248a1f1bed10239/.coderabbit.yaml#L1-L66) | Sourcery; CodeRabbit on demand | hosted | Sourcery on open; CodeRabbit on `@coderabbitai review` | anyone who can comment | none in repo | n/a | comment |
| [LLVM](https://github.com/llvm/llvm-project/blob/c80ecb8de257a4deb2bec994e81e9b942425e6f6/.github/instructions/llvm.instructions.md#L1-L8) | Copilot review with path-scoped instructions | hosted | on request | PR authors | none in repo | n/a | comment |
| [SGLang](https://github.com/sgl-project/sglang/pull/50748) | Claude (app, manual mode) | hosted | `@claude review` | maintainers | none in repo | n/a | comment |
| [Zephyr](https://github.com/zephyrproject-rtos/zephyr/blob/1eabf946704285c7861e7aed1a20a8cf17be493f/.github/workflows/ai_review.yml#L3-L54) | zephyr-review-agent (OpenRouter) | workflow | `ai-review` label, then every push while it stays | whoever can label | API key on `pull_request_target` | yes | comment |
| [Airflow](https://github.com/apache/airflow/blob/8894f5be87f056f69f7d847ab0e3e394669635eb/.github/instructions/code-review.instructions.md#L1-L10) | Copilot review with review instructions | hosted | on request | PR authors | none in repo | n/a | comment |
| [Gutenberg](https://github.com/WordPress/gutenberg/blob/a6d3dd98db4fb870ddeac4c3c1706e03efd79c1b/.coderabbit.yaml#L1-L21) | CodeRabbit (auto off); LLM step that picks human reviewers | hosted + workflow | `@coderabbitai review`; routing on every PR | anyone | routing API key on `pull_request_target`, read-only token | yes | comment; request human reviewers |
| [Doris](https://github.com/apache/doris/blob/3c559f7a52919ee782099045ad85c3be31b684d9/.github/workflows/code-review-runner.yml#L3-L36) | Codex CLI | workflow | `/review` comment, or manual dispatch | members, owners, collaborators | storage keys holding Codex auth, in the job that checks out the PR head | no | comment, request changes, commit status |
| [ExecuTorch](https://github.com/pytorch/executorch/blob/ec5e3cf21fec4175bf6dce3f09f96a8111481bdd/.github/workflows/claude-code.yml#L3-L31) | claude-code-action via a shared workflow | workflow | `@claude` mention | members, owners, collaborators with write access | AWS Bedrock by OIDC | mixed | comment |

### What each project did on the four questions

**How reviews are triggered.** Most hosted apps run on every pull request;
the rest wait for a comment command or a review request. Among the 8
repositories whose workflows run or start an AI step, Zephyr, Gutenberg's routing step, systemd and Envoy run on
`pull_request_target`; Doris, ExecuTorch and systemd's `@claude review` run on
comment events; OpenWrt and Home Assistant keep the model step off pull
request events altogether, with a schedule and a `workflow_run` hand-off.
OpenWrt moved to the schedule after its per-pull-request trigger
[used up the daily run quota](https://github.com/openwrt/openwrt/commit/4871040d5).

**What the bot can do.** Almost everywhere the bot can only comment. There
are two exceptions.

- [pnpm's Qodo configuration](https://github.com/pnpm/pnpm/blob/b0a7dac0fe88264cdef0f258cdb755d5d28c4a8f/.pr_agent.toml#L1-L49)
  auto-approves small changes, and CodeRabbit and Qodo approvals add a label
  through
  [a workflow](https://github.com/pnpm/pnpm/blob/b0a7dac0fe88264cdef0f258cdb755d5d28c4a8f/.github/workflows/pr-review-automation-privileged.yml#L83-L92).
  The ruleset requires 0 approvals, so the label is a signal for the
  maintainer and nothing more.
- [Doris](https://github.com/apache/doris/blob/3c559f7a52919ee782099045ad85c3be31b684d9/.github/workflows/code-review-runner.yml#L778-L784)
  requests changes on its two highest severities and posts a `code-review`
  commit status. Whether that status is required is not readable.

systemd's bot resolves its own review threads, which matters in a repository
that requires every thread resolved before merge. No bot in the sample pushes
commits or merges.

**How secrets and write access stay away from untrusted pull requests.** The
12 hosted reviewers keep their credentials off Actions entirely. Among the 8
repositories whose workflows run or start an AI step:

- **Who holds the model credential.** Two assume an AWS role by OIDC, so no
  long-lived key exists (systemd and ExecuTorch, both on Bedrock). Home
  Assistant uses the `copilot-requests` permission instead of a key. Envoy's
  workflow holds no credential at all. The other four keep a static key or
  token in the job: Zephyr's OpenRouter key, Doris's storage keys, Gutenberg's
  routing key, and OpenWrt's token for starting its routine.
- **What the model reads.** Zephyr, Gutenberg, OpenWrt and Envoy check out the
  base branch or nothing, and read the pull request through the API. Doris and
  systemd check out the pull request head in the job that runs the model.
  Doris accepts the trigger only from members. systemd gives that job a
  read-only token and runs the agent in Claude Code's sandbox, and
  [a separate job](https://github.com/systemd/systemd/blob/d26f3cbfbfb52882012803a15be869b112d55539/.github/workflows/claude-review.yml#L1010-L1040)
  posts the comments.
- **Pinning.** Zephyr, Home Assistant, Gutenberg and systemd pin every action
  by commit SHA. OpenWrt, Doris and ExecuTorch use version tags or call a
  shared workflow at `@main`.

**What a human still decides.** Humans hold every gate on a merge, and where
the gate is visible the rules say so: Superset's
[`.asf.yaml`](https://github.com/apache/superset/blob/c01faa7ac1b2e3dd2e6b1cf4f233c0b5d888bb16/.asf.yaml#L72-L109)
requires one code-owner approval, Prometheus requires
[two humans on every change](https://github.com/prometheus/prometheus/blob/cb4e01dd1cb1673d4bcde6f3a16fb767b7f8eb88/CONTRIBUTING.md#L76-L83),
bot-authored ones included. Rust's
[LLM policy](https://forge.rust-lang.org/policies/llm-usage.html) says an LLM
review may not stand in for a human review. Projects that write about AI
review comments put the human between the model and the thread: Zephyr's
[`AGENTS.md`](https://github.com/zephyrproject-rtos/zephyr/blob/1eabf946704285c7861e7aed1a20a8cf17be493f/AGENTS.md#L17-L32)
says a human verifies AI-assisted review comments before posting, and
Envoy's
[policy](https://github.com/envoyproxy/envoy/blob/fdd02e6b8cbc7fe7e0b6c628e87623a8cb09da50/CONTRIBUTING.md#L41-L80)
asks reviewers who use AI to understand its comments and remove hallucinated
or low-value ones.

## Risks seen in the wild

These come from the repositories' own history and configuration. They describe
patterns; none of them is a claim that a project's current workflow is
exploitable.

1. **A trigger path without an author check.** The reference case,
   jkoelker/schwab-mcp, runs `dobbyphus/action` with a mention path and a label
   path. Its `pull_request_review` trigger had no
   `OWNER`/`MEMBER`/`COLLABORATOR` check while the other mention triggers did,
   and the action ran from `@main`.
   [PR #239](https://github.com/jkoelker/schwab-mcp/pull/239) added the check
   and pinned the action; it references GHSA-m4cm-h832-2mgx. One workflow with
   several triggers needs the gate on every one.
2. **A gate that outlives one review.** A label that only someone with triage
   rights can apply is a reasonable gate, but it can be checked once or every
   time. Zephyr's
   [workflow](https://github.com/zephyrproject-rtos/zephyr/blob/1eabf946704285c7861e7aed1a20a8cf17be493f/.github/workflows/ai_review.yml#L3-L26)
   and systemd's
   [workflow](https://github.com/systemd/systemd/blob/d26f3cbfbfb52882012803a15be869b112d55539/.github/workflows/claude-review.yml#L26-L69)
   run on every push while the label stays on the pull request. schwab-mcp's label path
   [removes the label](https://github.com/jkoelker/schwab-mcp/blob/8232943afad9fb0a86489c7195da9a8e0f09604d/.github/workflows/agent.yaml#L137-L172)
   after each run, so new code needs a new decision from a maintainer.
3. **A model credential next to untrusted text.** On `pull_request_target`
   the job sees secrets even for a fork's pull request. Three workflows keep a
   static model key or token in a job that handles pull request content
   (Zephyr, Doris, Gutenberg). Prompt injection through the diff can then steer a model that holds that key, and, where the
   token can write, the repository. The two OIDC designs and Home Assistant's
   permission-based Copilot access avoid a long-lived key; systemd also keeps
   the model job read-only.
4. **Cost and quota.** OpenWrt dropped its per-pull-request trigger after it
   [exhausted 15 routine runs a day](https://github.com/openwrt/openwrt/commit/4871040d5).
   Home Assistant added
   [credit-exhaustion handling and timeouts](https://github.com/home-assistant/core/pull/190356).
   Copilot reviews that hit a quota post an empty notice, as in conda-forge.
5. **Unpinned dependencies.** Three workflows run actions by tag or call a
   shared workflow at `@main`, so code that runs next to the credential can
   change without a commit in the repository. Zephyr changed its checkout
   ref after an
   [OpenSSF Scorecard finding](https://github.com/zephyrproject-rtos/zephyr/commit/03301c5a8).
6. **Noise.** vLLM posts a placeholder
   review on every fork pull request; SGLang's app posts a manual-mode notice
   on most; KubeVirt's CodeRabbit posts "Review skipped" on nearly every pull
   request. Projects rolled automation back when it was noisy: Envoy
   [turned off Gemini's automatic reviews](https://github.com/envoyproxy/envoy/commit/37edcc2b0)
   and kept manual ones, Gitea's Copilot ruleset is
   [disabled](https://github.com/go-gitea/gitea/rules/19468196), and a Valkey
   [pull request](https://github.com/valkey-io/valkey/pull/4975) proposes
   retiring its Copilot instructions.
7. **Bot approvals as signals.** In pnpm, Qodo approves small changes and the
   approval adds a label. With no required approvals, a reader can mistake the
   bot's green check for review.
8. **Policy conflict.** NumPy, OpenTelemetry, DuckDB and Servo forbid
   AI-written comments in review threads; Rust requires a separate account
   users can block. A bot that posts as the maintainer, or posts unlabeled,
   would break these rules in projects like them.

## Recommendation for t3-codemode

t3-codemode has one maintainer, merges by rebase, and protects `main` with a
ruleset: CI and DCO checks required, 0 approvals required, and every review
thread resolved before merge. Its CI pins actions by tag. The maintainer
already runs a local AI review before opening a pull request. The reviewer should
add a second, independent read on the pull request itself without becoming a
second way into the repository.

The recommended setup is a label-triggered, three-job workflow that runs Claude
Code headless and only advises.

1. **Trigger.** An `ai-review` label, plus `workflow_dispatch` with a pull
   request number. The workflow checks the labeler's permission through the
   API and accepts write or admin. It removes the label after every run, so a
   new push needs a new label (risk 2). There is no mention or comment
   trigger, so the only two ways in are the label, behind the permission
   check, and manual dispatch, which GitHub allows only to accounts with write
   access (risk 1). The workflow skips draft pull requests.
2. **Three jobs, after systemd.**
   - `setup` reads the pull request through the API and writes the diff and
     metadata to an artifact. It has `pull-requests: read` and no model
     credential.
   - `review` checks out the base commit with `persist-credentials: false`,
     reads the diff as data, and runs Claude Code headless with read-only
     tools, a turn limit and a timeout. Its token has `contents: read` and
     nothing else. It holds an Anthropic API key from a dedicated Anthropic
     workspace with a spend limit, stored in a GitHub environment that only
     this job uses. It never
     builds or runs the pull request's code. Its output is a JSON file.
   - `post` has `pull-requests: write` and no model credential. It validates
     the JSON, posts one review with `event: COMMENT`, and removes the label,
     even when `review` fails.
3. **Powers.** Comment only: no approve, no request changes, no commits, no
   labels other than removing `ai-review`. Because `main` requires every
   thread resolved, the bot posts inline threads only for must-fix findings
   and puts the rest in the review summary, so a nit cannot block a merge
   (risk 6).
4. **Supply chain.** Every `uses:` pinned by full commit SHA, the Claude Code
   version pinned and checked against a hash, `permissions: {}` at the top of
   the workflow, a concurrency group per pull request (risk 5).
5. **Prompt and configuration in the repository.** The prompt lives in
   `.github/ai-review/` and points the model at `.github/CONTRIBUTING.md` and the
   repository's rules, so changing what the bot looks for is a reviewed
   commit.
6. **Clear identity.** The review states that it is AI output and names the
   model, so a reader knows which comments came from a model.
   `.github/CONTRIBUTING.md` gets a short section: what the reviewer is, that it is
   advisory, and who can start it.

**The alternatives, and why they lost.**

- **A hosted app on every push** (Backstage, pnpm) is the lowest effort and
  keeps keys out of the repository, which is why 12 of the 19 project-run
  reviewers are hosted. Here it would review every push, and most pull
  requests are the maintainer's own and already reviewed locally. Because
  `main` requires every thread resolved, each inline comment it left would
  hold the merge until someone resolved it. It also leaves the prompt and the
  model choice outside the repository. Copilot
  review through a ruleset stays the fallback if the workflow proves too
  costly to keep.
- **A mention-triggered agent with a write token** (the schwab-mcp mention
  path) is where the reference case's incident happened, and no reviewer in
  the sample was seen pushing a commit or merging, so a review has no use for
  the write token.
- **Automatic review on every pull request** (OpenWrt's first design) costs
  more with every push, and two projects in the sample had to add quota and
  credit limits after the fact. The label can be added automatically to the
  maintainer's own pull requests later, as systemd does for members, once the cost per review is
  known.
- **An API key, for now.** The two OIDC designs run on AWS Bedrock.
  t3-codemode has no cloud account, and a key from a spend-limited workspace,
  in a job with a read-only token, behind a write-access label, is the smaller change. Moving to Bedrock by OIDC later removes the long-lived key.

## Reproducing

The selection queries, the community owner list, the window and the ranking
rule are in the data file under `survey`; the ranked candidates are under
`candidates`. Rerunning the same queries later returns different counts,
because merge activity moves. The run date, 2026-11-18, fixes the snapshot
this survey describes.
