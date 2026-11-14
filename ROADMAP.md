# Roadmap

RolidSCP moves your files, often on servers that matter. So reliability comes first and new features
second: the 0.8 series gets no new features, only fixes and stress tests of what is already there.
Small new features come in 0.9, large ones from 1.0 alpha.

This is a plan, not a promise. There are no dates, and the order within a section can change.
Releases are listed on the [Releases](https://github.com/MrRolid/RolidSCP/releases) page.

## 0.8.x: hardening

No new features. Each release tests one area under load, fixes what breaks and adds tests that would
catch it again.

- **0.8.15 Many files:** a folder with a million files, deep folder trees, memory use, sorting,
  search, folder sizes, copying and deleting millions of small files. Unusual file names (invalid
  UTF-8, line breaks, different Unicode forms) and special files such as pipes, sockets and devices.
- **0.8.16 Big files:** files over 100 GB tested for real. Terabytes and petabytes are tested with a
  **simulated** data source, not with real data: counters, time estimates, resuming and sparse files.
  Also a full disk, disk quotas and transfers running overnight. The program killed or the power
  lost in the middle of every operation: no data may be lost, and settings are written safely.
- **0.8.17 Bad connections:** high latency, jitter, packet loss, changing speed, repeated drop-outs
  and frozen connections. Every transfer in these tests is verified with a checksum. The computer
  going to sleep and waking up during a session.
- **0.8.18 Servers and distributions:** different OpenSSH versions, Dropbear, SFTP-only servers
  without a shell, BusyBox, NAS devices and Windows servers; Debian, Ubuntu, Linux Mint, Fedora,
  openSUSE and Arch; X11 and Wayland; high-DPI screens. A broken or hostile server (odd names, huge
  or false listings, links pointing outside the target folder), permissions, ACLs and extended
  attributes, and time zones and server clocks that are off.
- **0.8.19 Polish:** consistent controls in every dialog, fuzz testing of everything the program reads
  (archives, server listings, `known_hosts`), a licence check of all dependencies and a check of the
  code for matches with other projects, and performance measured on every build. Two copies of the
  program running at once; settings from older versions, going back to an older version and damaged
  settings files. Keyboard-only use, screen readers and long translated texts. Signed checksums of
  the release files and reproducible builds.

## 0.9.x: small new features

- Settings in a window of their own, with a tree of sections and a filter.
- Ignored files by masks.
- Dark theme and color themes built on one palette.
- Display density: icon size, row height, font.
- Layout: show or hide parts of the window (hiding the toolbar and the bar above the panels already
  works).
- Your own viewer, file comparison tool and terminal (your own editor already works).
- Mouse and keyboard behavior.
- `~/.ssh/config`.
- Trash for local deleting.
- Custom keyboard shortcuts.
- Custom columns: which columns, in which order.
- A log of operations you can browse.
- Import of saved sites from WinSCP and FileZilla.

Already available: ssh-agent and passphrase-protected keys, colors by file type, an external editor,
and "Open with" associations per file type.

## 1.0 alpha and later: large features

- A Windows version from the same code, with the system-specific parts in separate modules.
- File sharing in the local network, compatible with the LocalSend protocol.
- Connecting through a jump host (ProxyJump).
- A transfer queue that survives a restart of the program.
- Comparing and synchronizing folders.
- Renaming many files at once.
- A customizable toolbar.
- A built-in editor.
- Sharper text: letter positions on whole pixels.

Then beta, release candidates and 1.0.

## Not planned

- Plugins in the style of Double Commander or Total Commander (archive, file system, viewer and
  content plugins).
- A Flatpak package. RolidSCP is offered as deb, rpm and AppImage.

## Suggesting a feature

Open an issue: [github.com/MrRolid/RolidSCP/issues/new](https://github.com/MrRolid/RolidSCP/issues/new).
Describe what you want to do and why. A real situation from your work helps more than a list of
features from another program. Bugs, and above all anything that could lose data, go before new
features.
