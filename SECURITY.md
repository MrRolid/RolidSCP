# Security policy

## Supported versions

Only the latest released version of RolidSCP receives security fixes. Please check that the problem
still exists in the latest release before reporting it.

## Reporting a vulnerability

Please report vulnerabilities **privately**, not in a public issue:

- on GitHub: the repository's **Security** tab → **Report a vulnerability**, or
- by e-mail to **martin@vorel.eu**.

A good report contains:

- the RolidSCP version (Help → About) and the Linux distribution,
- what the problem is and what an attacker could do with it,
- steps to reproduce it, ideally the smallest possible case,
- logs or a proof of concept, if you have them (please remove real passwords, keys and server names).

## What happens next

- You will get a confirmation that the report was received within **7 days**.
- The problem is investigated and fixed in a new release.
- Details of the vulnerability are published only **after the fixed version is released**. If you
  wish, you will be credited in the release notes.

## Scope

In scope:

- the RolidSCP application itself (local and SFTP file operations, archives, the built-in viewer
  and editor integration),
- storage of saved passwords (the encrypted vault and its master password),
- server verification (the `known_hosts` check and host key handling),
- the terminal password helper (the one-time `SSH_ASKPASS` helper used when opening a terminal on a
  server).

Out of scope:

- bugs in third-party SSH/SFTP servers or other software RolidSCP talks to,
- problems that require an attacker who already controls the user's account or computer.
