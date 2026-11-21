# Contributing

Bug reports, fixes, tests, documentation, ports to other platforms and new
upstream integrations are welcome. Start with the
[contribution guide](https://github.com/OttoOps/t3-codemode/blob/main/.github/CONTRIBUTING.md).
It covers the DCO signoff every commit carries (`git commit -s`), how pull
requests merge, and what the project will not take.

Report vulnerabilities privately through the
[security policy](https://github.com/OttoOps/t3-codemode/blob/main/.github/SECURITY.md).
Participation follows the
[Code of Conduct](https://github.com/OttoOps/t3-codemode/blob/main/.github/CODE_OF_CONDUCT.md).
The code is under the
[MIT license](https://github.com/OttoOps/t3-codemode/blob/main/LICENSE).

## Checks

Run the checks CI runs, from the repository root:

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
python3 -m unittest discover -s tests/skills -v
shellcheck deploy/*.sh              # when you change an installer
actionlint                          # when you change a workflow
```

To change the installers or workflows, also install
[ShellCheck](https://www.shellcheck.net/) and
[actionlint](https://github.com/rhysd/actionlint). `deploy/install.ps1` must run
on Windows PowerShell 5.1. The contribution
guide shows how to try `install.sh` against a scratch `HOME`.

The test suite runs against an in-process fake T3 and fake MCP servers, so most
work needs no T3 Code server. `tests/fixtures/t3-tools.json` is a snapshot of
T3's 74 tools. At runtime the catalog comes live from `tools/list`, refreshed
every 10 minutes and cached in the store for when T3 is down.

A hidden `bench` command measures latency and RSS against the in-process fake
T3:

```bash
t3cm bench -n 1000 --tools tests/fixtures/t3-tools.json
```

## This site

`docs/build.sh` generates the [API reference](api.md) page from `t3cm types`,
then builds the site into `docs/book/` with mdBook 0.5.4. CI builds it on every
pull request, checks its links with lychee, and deploys it to GitHub Pages from
`main`. The pages are Markdown under `docs/src/`, and `docs/src/SUMMARY.md`
lists them in order.
