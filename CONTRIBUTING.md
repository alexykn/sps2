# Contributing to sps

> We love merge requests! This guide shows the fastest path from **idea** to **merged code**. Skip straight to the *Quick‑Start* if you just want to get going, or dive into the details below.

---

## ⏩ Quick‑Start

### 1. Fork, clone & branch
```bash
git clone https://github.com/<your-username>/sps.git
cd sps
git checkout -b feat/<topic>
```

### 2. Compile fast
```bash
cargo check --workspace --all-targets
```

### 3. Format (uses nightly toolchain)
```bash
cargo fmt --all
```

### 4. Lint
```bash
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

### 7. Commit (Conventional + DCO)
```bash
git commit -s -m "feat(core): add new fetcher"
```

### 8. Push & open a Merge Request against `main`
```bash
git push origin feat/<topic>
# then open a merge request on GitHub
```

-----

## Coding Style

 * Complete Spec in [README.md](https://github.com/alexykn/sps2)

-----

## Git & Commits

  * **Fork** the repo on GitHub and add your remote if you haven’t already.
  * **Branches**: use feature branches like `feat/…`, `fix/…`, `docs/…`, `test/…`.
  * **Conventional Commits** preferred (`feat(core): add bottle caching`).
  * **DCO**: add `-s` flag (`git commit -s …`).
  * Keep commits atomic; squash fix‑ups before marking the MR ready.

-----

## Merge‑Request Flow

1.  Sync with `main`; no rebase.
2.  Ensure your code is formatted correctly with `cargo fmt --all`.
3.  Ensure CI is green (build, fmt check, clippy, tests on macOS using appropriate toolchains).
4.  Fill out the MR template; explain *why* + *how*.
5.  Respond to review comments promptly – we’re friendly, promise!
6.  Maintainers will *Squash & Merge* (unless history is already clean).

-----

## Reporting Issues

  * **Bug** – include repro steps, expected vs. actual, macOS version & architecture (Intel/ARM).
  * **Feature** – explain use‑case, alternatives, and willingness to implement.
  * **Security** – email maintainers privately; do **not** file a public issue.

-----

## License & DCO

By submitting code you agree to the BSD‑3‑Clause license and certify the [Developer Certificate of Origin][Developer Certificate of Origin].

-----

## Code of Conduct

We follow the [Contributor Covenant][Contributor Covenant]; be kind and inclusive. Report misconduct privately to the core team.

-----

Happy coding – and thanks for making sps better! ✨

[rustup.rs]: https://rustup.rs/
[Rust API Guidelines]: https://rust-lang.github.io/api-guidelines/
[Developer Certificate of Origin]: https://developercertificate.org/
[Contributor Covenant]: https://www.contributor-covenant.org/version/2/1/code_of_conduct/
