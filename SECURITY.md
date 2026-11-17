# Security policy

## Reporting a vulnerability

Please report it privately. On this repo's **Security** tab, choose **Report a vulnerability**, which uses GitHub's private vulnerability reporting. Please don't open a public issue.

You'll get a reply as soon as the maintainer can look at it. Fixes are released as a new version.

## Supported versions

There is no release yet, so only the `main` branch is supported. After releases begin, only the latest release will get security fixes.

## Scope

In scope:
- The `drainlogd` background agent. It runs as your user with no root, reads `pmset` and IOKit, and writes `~/Library/Application Support/Drainlog/drainlog.db`.
- The Drainlog app, which reads that database.
- The build and release scripts in this repo.

Out of scope: macOS's own power-management tools and logs.
