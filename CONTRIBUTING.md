Contributing to Neovim
======================

Getting started
---------------

Read [:help dev-quickstart](runtime/doc/dev_tools.txt) and [BUILD.md](./BUILD.md)
to see how to build, run tests, and start hacking on the codebase.

Reporting problems
------------------

- [Check the FAQ][wiki-faq].
- [Search existing issues][github-issues] (including closed!)
- Update Neovim to the latest version to see if your problem persists.
- Try to reproduce with `nvim --clean` ("factory defaults").
- If a specific configuration or plugin is necessary to recreate the problem, use the minimal template in `contrib/minimal.lua` with `nvim --clean -u contrib/minimal.lua` after making the necessary changes.
- [Bisect](https://neovim.io/doc/user/starting.html#bisect) your config: disable plugins incrementally, to narrow down the cause of the issue.
- [Bisect][git-bisect] Neovim's source code to find the cause of a regression, if you can. This is _extremely_ helpful.
- When reporting a crash, [include a stacktrace](https://neovim.io/doc/user/dev_tools.html#dev-tools-backtrace).
- Check the logs: `:edit $NVIM_LOG_FILE`.
- Include `rustc --version` and `cargo --version` for build-related issues.

Developer guidelines
--------------------

- Core editor code in `src/nvim/` is written in Rust (`edition = "2024"`). High-level features and plugins should generally be implemented in Lua (`runtime/lua/vim/`).
- Read [:help dev-quickstart](runtime/doc/dev_tools.txt) to see how to run tests and start hacking on the codebase.
- Read [:help dev](runtime/doc/dev.txt) and [:help dev-arch](runtime/doc/dev_arch.txt) if you are working on Nvim core.
- Read [:help dev-ui](https://neovim.io/doc/user/dev.html#dev-ui) if you are developing a UI.
- Read [:help dev-api-client](https://neovim.io/doc/user/dev.html#dev-api-client) if you are developing an API client.

Pull requests (PRs)
---------------------

### Guidelines

- Don't ask to be assigned to an issue, just send a (reasonably complete) PR and
  mark it as Draft until it is ready for review.
- Your PR must include [test coverage][run-tests].
- Avoid cosmetic changes to unrelated files in the same commit.
- Use a [feature branch][git-feature-branch] instead of the `master` branch.
- Use a _rebase workflow_. After addressing review comments, it's fine to force-push.

### AI-assisted work

Using AI for contributions is acceptable, given the following (see also [AGENTS.md](./AGENTS.md)):

- If AI was used for a commit, add a generic `AI-assisted` trailer to the commit message. Do NOT name a specific model or AI service.
- YOU review the output before sending a non-Draft PR. Do NOT request review
  until YOU have checked the AI-generated PR and verified the following:
- REMOVE verbosity and blathering from documentation, comments, PR description,
  commit message, etc. All resources, including names, should be CONCISE and
  CLEAR. They should contain USEFUL information and nothing more.
- REMOVE and DEDUPLICATE redundant code, tests, explanations, etc. Explicitness
  and clarity are GOOD but verbosity, over-explanation, and redundancy is BAD.

### Merging to master

For maintainers: when a PR is ready to merge to master,

- prefer _Squash Merge_ for "single-commit PRs" (when the PR has only one meaningful commit).
- prefer _Merge_ for "multi-commit PRs" (when the PR has multiple meaningful commits).

### Stages: Draft and Ready for review

Pull requests have two stages: Draft and Ready for review.

1. [Create a Draft PR][pr-draft] while you are _not_ requesting feedback and
   still working on the PR.
2. [Change your PR to Ready][pr-ready] when the PR is ready for review.
    - You can convert back to Draft at any time.

Do __not__ add labels like `[RFC]` or `[WIP]` in the title to indicate the
state of your PR: this just adds noise.

### PR description

For bugfixes, your PR title should be essentially the same as (1) the
"Problem" statement and (2) the test-case name.

### Commit messages

Follow the [conventional commits guidelines][conventional_commits] to *make reviews easier* and to make
the VCS/git logs more valuable (run `cargo xtask lintcommit`). The structure of a commit message is:

    type(scope): subject

    Problem:
    ...

    Solution:
    ...

- Commit message **subject**:
    - Prefix with a [_type_](https://github.com/commitizen/conventional-commit-types/blob/master/index.json):
        - `build ci docs feat fix perf refactor revert test`
    - Append an optional `(scope)` such as `(lsp)`, `(treesitter)`, `(float)`, …
    - Use the _imperative voice_ and start the description with a lowercase word (`fix(window): handle split resize` rather than `fix(window): Fixed split resize.`).
    - Keep it short (under 80 characters, no trailing period).
- Commit message **body** (detail):
    - Concisely describe the Problem/Solution in the commit **body**.
- Indicate breaking API changes with `!` after the type/scope, and a `BREAKING CHANGE` footer.

### Automated builds (CI)

Each pull request must pass the automated builds on [GitHub Actions]:

- `cargo fmt --all -- --check` and `cargo xtask lintrust` (`cargo clippy --workspace --all-targets -- -D warnings`) must pass with 0 warnings.
- Rust unit tests (`cargo xtask unittest`), functional tests (`cargo xtask functionaltest`), and legacy tests (`cargo xtask oldtest`) must pass.
- Documentation generation (`cargo xtask doc`) and linters (`cargo xtask lint`) must pass cleanly.

Coding
------

### Lint & Format

You can run all linters locally via `cargo xtask`:

```bash
cargo xtask lint        # runs lintrust, lintlua, linterrcodes, and lintdoc
cargo xtask lintrust    # runs cargo clippy --workspace --all-targets -- -D warnings
cargo xtask lintlua     # runs stylua --check
cargo xtask linterrcodes
cargo xtask lintdoc
cargo xtask lintcommit
```

And format Rust and Lua code via:

```bash
cargo xtask format      # runs cargo fmt and stylua
```

### Navigate

- Use **[rust-analyzer]** for Rust IDE support and **[lua-language-server]** / **[EmmyLua]** for Lua files.
- Set `blame.ignoreRevsFile` to ignore noisy formatting commits in `git blame`:
  ```bash
  git config blame.ignoreRevsFile .git-blame-ignore-revs
  ```

### Lua runtime files

The Lua [`runtime/lua/vim/_core/`](./runtime/lua/vim/_core/) modules can be tested directly during development by starting Nvim with `--luamod-dev` and `$VIMRUNTIME`:

```bash
VIMRUNTIME=./runtime ./target/nvim-build/bin/nvim --luamod-dev
```

Documentation
-------------

Read [:help dev-doc][dev-doc-guide] to understand the expected documentation style and conventions.

### Generating :help

Many `:help` docs are autogenerated from Rust and Lua docstrings/metadata. To regenerate the documentation run:

```bash
cargo xtask doc
```

To validate the documentation files, run:

```bash
cargo xtask lintdoc
```

Main documentation generator files:
- `./src/gen/gen_vimdoc.lua`: Parses Rust/C-style and Lua docstrings to render vimdoc files.
- `./src/gen/gen_eval_files.lua`: Generates documentation and Lua type files from metadata (`runtime/lua/vim/*`, `src/nvim/api/*`, `src/nvim/eval.lua`, `src/nvim/options.lua`).
- `./src/gen/gen_help_html.lua`: Validates `:help` tags, syntax, and links (`cargo xtask lintdoc`).

Third-party dependencies
------------------------

Bundled C dependencies (`libuv`, `LuaJIT`, `Lua`, `luv`, `lua-compat-5.3`, `tree-sitter`, and bundled tree-sitter parsers) are specified in `deps.txt` and built by `src/deps_build.rs`.
To force a clean rebuild of `.deps`:

```bash
cargo xtask deps --force
```

Set `DEPS_IGNORE_SHA=1` in the environment when testing a modified dependency archive URL in `deps.txt`.

[GitHub Actions]: https://github.com/freddiehaddad/neovim/actions
[conventional_commits]: https://www.conventionalcommits.org
[dev-doc-guide]: https://neovim.io/doc/user/dev.html#dev-doc
[EmmyLua]: https://github.com/EmmyLuaLs/emmylua-analyzer-rust
[git-bisect]: https://git-scm.com/book/en/v2/Git-Tools-Debugging-with-Git
[git-feature-branch]: https://www.atlassian.com/git/tutorials/comparing-workflows
[github-issues]: https://github.com/freddiehaddad/neovim/issues
[lua-language-server]: https://github.com/LuaLS/lua-language-server
[pr-draft]: https://docs.github.com/en/pull-requests/collaborating-with-pull-requests/proposing-changes-to-your-work-with-pull-requests/creating-a-pull-request
[pr-ready]: https://docs.github.com/en/github/collaborating-with-pull-requests/proposing-changes-to-your-work-with-pull-requests/changing-the-stage-of-a-pull-request
[run-tests]: ./runtime/doc/dev_test.txt
[rust-analyzer]: https://rust-analyzer.github.io/
[wiki-faq]: https://neovim.io/doc/user/faq.html
