# Contributing to Compio

Thanks for your help improving the project! We are so happy to have you! :tada:

There are opportunities to contribute to Compio at any level. It doesn't matter if
you are just getting started with Rust or are the most weathered expert, we can
use your help. If you have any question about Compio, feel free to join [our group](https://t.me/compio_rs) in telegram.

This guide will walk you through the process of contributing to Compio on following topics:

- [General guidelines](#general-guidelines)
- [Contribute with issue](#contribute-with-issue)
- [Contribute with pull request](#contribute-with-pull-request)

## General guidelines

We adhere to [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct). tl;dr: **be nice**. Before making any contribution, check existing issue and pull requests to avoid duplication of effort. Also, in case of bug, try updating to the latest version of Compio and/or rust might help.

## Contribute with issue

If you find a bug or have a feature request, please [open an issue](https://github.com/compio-rs/compio/issues/new/choose) with detailed description. Issues that are lack of informaton or destructive will be requested for more information or closed.

It's also helpful if you can provide the following information:

- A minimal reproducible example
- The version of Compio you are using.
- The version of Rust you are using.
- Your environment (OS, etc).

## Contribute with pull request

We welcome any code contributions. It's always welcome and recommended to open an issue to discuss on major changes before opening a PR. And please the following guidelines below:

- All PRs need to pass CI tests and style check. You can run `cargo test --all-features` and `cargo clippy  --all-features` locally to check.
- All PRs need to be reviewed by at least one maintainer before getting merged.
- Commit messages should follow [Angular Convention](https://github.com/angular/angular/blob/main/CONTRIBUTING.md#-commit-message-format).
- In PR body, a description of what it does and why it is needed should be provided.
