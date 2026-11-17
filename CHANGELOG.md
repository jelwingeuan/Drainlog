# Changelog

All notable changes to this project are documented here. The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- Rust core: ingests `pmset` and IOKit data, builds an interval ledger (asleep, DarkWake, awake), prices each night with labelled measured and estimated parts and an explicit unexplained line, ranks causes, and gives a "can't tell" verdict.
- `drainlogd` background agent that records battery state at sleep, at wake and on a timer.
- SwiftUI app: morning report window, menu bar popover and morning notification.
- Replay test suite with synthetic fixtures, plus `judge/` tools for the 10-night check.
- App icon and logo.
