# Security policy

## Reporting a problem

Please don't open a public issue for a security problem. Report it privately through
[GitHub's private vulnerability reporting](https://github.com/tylort69/eve-majRusty/security/advisories/new) and include
the steps to reproduce it, the version, and what someone could do with it. You'll get a reply as soon as possible, and
credit in the fix's release notes if you'd like it.

## Supported versions

Fixes go into the latest release only.

## What's in scope

EVE-Maj Rusty runs locally. The settings window talks to its own backend on `127.0.0.1` with a per-launch token, the
app takes commands over a per-user socket in `$XDG_RUNTIME_DIR` and through `evemajpreview://` links, and it checks
GitHub for releases. Problems in any of these (for example, another local user or a web page being able to control
the app or read its settings) are in scope.
