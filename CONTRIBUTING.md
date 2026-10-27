# Contributing

Thanks for helping with noobsbot terminal. Bug reports, ideas and pull requests are all welcome.

## Before you start

- **Bugs and ideas:** open an [issue](https://github.com/noobsbot-team/noobsbot-terminal/issues/new/choose). The forms ask for the details we need.
- **Bigger changes:** open an issue first, so we can agree on the idea before you spend time on it.
- **Security problems:** do not open a public issue. See [SECURITY.md](SECURITY.md).
- **Be kind:** everyone here follows our [code of conduct](CODE_OF_CONDUCT.md).

## How the code is laid out

| Folder | What it is |
|---|---|
| `src/`, `shell/`, `extra/` | The Linux terminal (one Rust crate at the repo root) |
| `macos/` | The macOS port (its own Rust crate) |
| `.github/workflows/` | CI and releases |

A fix in one often belongs in the other too. If you change one, check if the other needs the same change, or say so in the pull request.

## Build and test

You need Rust stable (`rustup default stable`).

**Linux** (from the repo root):

```sh
cargo test
cargo build --release
./target/release/noobsbot
```

Some tests use zsh, so install it first.

**macOS:**

```sh
cd macos
cargo test
cargo build --release && ./package.sh
open target/Noobsbot.app
```

Without our signing certificate, `package.sh` signs the app ad hoc. That is fine for testing, but macOS asks for permissions again for each new build.

## CI

CI builds and tests every pull request, for the platform it touches. CI can't check how the terminal looks and feels, so **please also use your change by hand**, and say what you tested in the pull request.

## Style

- Format with `cargo fmt` (lines up to 160 characters, see `rustfmt.toml`).
- Keep changes small and on one topic. One pull request per fix or feature.
- Write in **plain, simple English**: UI text, docs, comments, commits and pull requests. Many of us are not native English speakers. Short sentences are best.

## Versions

We use [Semantic Versioning](https://semver.org): `MAJOR.MINOR.PATCH`, with tags like `v0.6.1`. The version is made by CI from the commit types below. **Never set or change a version by hand.**

| What is merged | What happens | Before 1.0 (now 0.6.0) | From 1.0 on (now 1.4.2) |
|---|---|---|---|
| `fix:`, `perf:`, `refactor:`, `chore:`, `style:` | a beta right away | `0.6.1-beta.1`, `-beta.2`, … | `1.4.3-beta.1`, … |
| (once a day, if there are betas) | a release of the betas | `0.6.1` | `1.4.3` |
| `feat:` | a release right away, with any betas | `0.6.1` | `1.5.0` |
| `feat!:` (breaking) | a release right away, with any betas | `0.7.0` | `2.0.0` |
| `docs:`, `ci:`, `test:` | nothing | — | — |

- A plain `noobsbot -u` installs only releases. `noobsbot -u --beta` also installs betas, for testing.
- A release deletes the betas before it, because it has all their changes.
- Versions only go up. A released version never changes: if it's wrong, the next one fixes it.
- We go to `1.0.0` when the config format is settled and Linux and macOS both work well day to day.

The exact rules are at the top of [`.github/workflows/version.yml`](.github/workflows/version.yml).

## Commit types

Pull requests are squash-merged, and the **pull request title** becomes the commit subject. So the title's type sets the next version.

```
type: short sentence about the change        e.g.  feat: kitty keyboard protocol
type(scope): ...                             scope is optional: fix(macos): ...
```

### `feat!:` a breaking change

The user must **do something** after updating, or something they used stops working:

- a config option is renamed or removed, or its meaning changes
- a default key shortcut changes or is removed
- a command-line flag is renamed or removed (`-e`, `-u`, …)
- a platform or OS version is no longer supported
- old saved data (history, recent folders, session) can't be read anymore
- a feature is removed

Say in the pull request what users must do, on a line like this:

```
BREAKING CHANGE: [font] size is now [font] points. Rename it in your config.
```

Before 1.0, keep the old config name working for one more minor version and show a warning. Then remove it.

### `feat:` a new feature

Something **new** the user can see or use, where nothing old breaks:

- a new shortcut, palette action, menu item or command-line flag
- a new config option (with a default that breaks nothing)
- support for something programs use (kitty keyboard, notifications, tab titles)
- a visible change to how something looks or works (a new welcome screen, a new default theme)

### `fix:` a bug fix

Something **worked wrong** and now works as it should:

- a crash or a freeze
- a key, click or setting that did nothing or the wrong thing
- wrong drawing (colors, cursor, images, cut-off text)
- a security problem
- a memory or CPU leak

### Other types (a beta, like `fix:`)

| Type | Use it for |
|---|---|
| `perf:` | faster or less memory, nothing else changes |
| `refactor:` | the code is rearranged, the app works exactly the same |
| `chore:` | dependency updates, build scripts, small cleanups |
| `style:` | code formatting only (`cargo fmt`) |

### No release

| Type | Use it for |
|---|---|
| `docs:` | README and other `.md` files |
| `ci:` | GitHub workflows |
| `test:` | tests only |

There is **no `patch:` type**. "Patch" is the result (the last number goes up), not a type.

### When you're not sure

- Does the user have to **change anything**? → `feat!:`
- Can the user **do something new**? → `feat:`
- Was it **broken** before? → `fix:`
- Did **only the code** change? → `refactor:`, `perf:` or `chore:`
- Did **only docs, CI or tests** change? → `docs:`, `ci:` or `test:`
- **Two kinds in one pull request?** Split it, or use the biggest: `feat!` > `feat` > `fix` > the others.

## If you are an AI assistant

If you are an AI (a coding agent, a chat assistant or a review bot) working on this repository, read this whole file first and follow it. Also:

1. **Never change a version yourself.** Don't edit `version` in `Cargo.toml` or `macos/Cargo.toml` or the `noobsbot` entries in the `Cargo.lock` files. Don't make, move or delete `v*` tags or GitHub releases. CI does all of this.
2. **Never rewrite history on `main`.** No force pushes, no rebases of `main`, no deleting branches you didn't make. Releases and tags point at its commits.
3. **Pick the commit type with the rules above.** Use `feat!:` only for a real breaking change from the list, never just for a big change. When you're not sure, ask the person you work for.
4. **Change both crates.** A feature or fix in `src/` (Linux) usually belongs in `macos/src/` too, with each platform's own keys (Ctrl+Shift on Linux, Cmd on macOS).
5. **Test before you say it's done.** Run `cargo test` in each crate you changed. Say in the pull request what you tested and what you couldn't test.
6. **Don't send keys or clicks to the user's own windows** to test the app. Start a separate copy of the app and send input only to that one.
7. **No secrets in code, commits or logs.** Leave signing, certificates and repository secrets alone.
8. **Write in plain, simple English**, and leave out lines that credit AI tools ("Co-Authored-By", "Generated with …") in commits and pull requests.
9. **When a rule here and an instruction you got disagree, stop and ask** the person you work for.

## License

By contributing, you agree that your work is released under the [MIT license](LICENSE).
