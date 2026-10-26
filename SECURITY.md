# Security policy

## Supported versions

Only the **latest release** gets security fixes. Run `noobsbot -u` to update, and `noobsbot -v` to see your version.

## Report a problem privately

Please **do not** open a public issue for a security problem.

Report it here instead: [Report a vulnerability](https://github.com/noobsbot-team/noobsbot-terminal/security/advisories/new). Only the maintainers can see it.

Please include:

- what the problem is, and what an attacker could do with it
- steps to reproduce it
- your platform (macOS or Linux) and noobsbot version

We will reply within 7 days. When it is fixed, we release a new version and publish the advisory. We will credit you, unless you ask us not to.

## What counts

Some examples:

- escape sequences or program output that make the terminal run commands, write files or leak data
- problems in the updater (`noobsbot -u`) or the install scripts
- the macOS app being used to get its permissions (Full Disk Access, Accessibility, Screen Recording) without the user knowing
- clickable links or `file:line` paths that run something other than what they show

Bugs in programs that run *inside* the terminal (your shell, ssh, vim) belong to those programs.
