# Contributing

Thanks for helping with noobsbot terminal. Bug reports, ideas and pull requests are all welcome.

## Before you start

- **Bugs and ideas:** open an [issue](https://github.com/noobsbot-team/noobsbot-terminal/issues/new/choose). The forms ask for the details we need.
- **Bigger changes:** open an issue first, so we can agree on the idea before you spend time on it.
- **Security problems:** do not open a public issue. See [SECURITY.md](SECURITY.md).

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

## CI does not run on pull requests

CI only runs after a merge into `main`. So **please build and test your change by hand** on the platform it touches, and say what you tested in the pull request.

## Style

- Format with `cargo fmt` (lines up to 160 characters, see `rustfmt.toml`).
- Keep changes small and on one topic. One pull request per fix or feature.
- Write in **plain, simple English**: UI text, docs, comments, commits and pull requests. Many of us are not native English speakers. Short sentences are best.

## Commit messages

Releases are made automatically once a day from the commits since the last release. The first word of the commit subject sets the new version:

| Subject starts with | Version change | Example |
|---|---|---|
| `feat!:` or contains `BREAKING` | major (1.0.0 → 2.0.0) | `feat!: new config format` |
| `feat:` | minor (0.5.3 → 0.6.0) | `feat: drop-down mode on macOS` |
| anything else | patch (0.5.3 → 0.5.4) | `fix(macos): clean app icon at small sizes` |

Pull requests are squash-merged, so the **pull request title** becomes the commit subject. Please give it the right prefix.

## License

By contributing, you agree that your work is released under the [MIT license](LICENSE).
