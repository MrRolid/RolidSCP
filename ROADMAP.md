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
- **0.8.16 Big files and marking:** what was tested for real and what was simulated:
  - Real: one file of 2 GiB copied to a local SFTP server and back through the program's copy code,
    a copy canceled and one killed in the middle, both continued from the part file, every copy
    compared by checksum. The planned test with a real file over 100 GB was not finished and moves
    to 0.8.17.
  - Simulated: files of 1 TiB, 1 PiB, 8 EiB and up to the largest size a file can have, from a
    synthetic data source whose content is computed instead of stored: resuming near the end, the
    counters, totals larger than the numbers can hold, the remaining time. A whole terabyte streamed
    through the copy code in memory.
  - Real: a full disk in the middle of a transfer on both sides (a small RAM disk, also behind the
    local SFTP server), and sparse files. A disk quota is simulated (the error a quota gives).
  - Real: the program killed (SIGKILL) at random moments of copying and moving to and from a server,
    uploading an edited file, packing, extracting, and saving the settings and the password vault.
  - Marking as in other file managers: Shift+click marks a range from the last clicked item, Shift
    with the arrows, Page Up, Page Down, Home and End extends it and going back unmarks only that
    range, a click without a key keeps the marks, Ctrl+A marks all and a second press unmarks.
    Measured in a folder of a million files.
- **0.8.17 Bad connections:** what was tested:
  - Transfers to and from a local SFTP server in a private network namespace whose line was made
    bad on purpose: high latency, jitter, packet loss, a speed changing every few seconds, repeated
    short and long drop-outs, a connection frozen for longer than the keepalive limit, and all of it
    at once. A big file and a folder of small files went up and back through the program's own
    copy code, and every file was compared by checksum. Fixed on the way: a slow line no longer
    drops a working connection, a transfer goes on as long as it gets further, and a file
    continued several times is counted once.
  - Also: the path above a panel is clickable part by part, bigger colored icons in the bar above
    the panels, the mask dialogs remember their options, packing and extracting clean up after a
    kill, the first key of the quick filter in a folder of a million files under 0.1 s, the news in
    the program translated into all eight languages, and release tags always annotated.
  - Not done, moved to 0.8.18: a real file over 100 GB copied to a server and back, a transfer
    running overnight with memory checks, and the computer going to sleep and waking up during a
    session.
- **0.8.18 Servers and distributions:** moved from 0.8.17: a real file over 100 GB copied to a
  server and back, interrupted and continued, a transfer running overnight with memory checks, and
  the computer going to sleep and waking up during a session. A line that is down for 30 seconds
  and up for only 8, again and again (the `flapping` scenario of `badnet.sh`), must be survived: a
  connection that receives no data for 15 seconds during a transfer is treated as dead and made
  again at once, reconnection is tried more often in the first minutes, and less of the end of a
  file is written again after a reconnection (now 4 MB). Downloading over a line with a long round
  trip is looked into: 24 s against 9.5 s for OpenSSH's `sftp` with 32 MiB at 100 ms, while
  uploading is as fast. Then different OpenSSH versions,
  Dropbear, SFTP-only servers without a shell, BusyBox, NAS devices and Windows servers; Debian,
  Ubuntu, Linux Mint, Fedora, openSUSE and Arch; X11 and Wayland; high-DPI screens. A broken or
  hostile server (odd names, huge or false listings, links pointing outside the target folder),
  permissions, ACLs and extended attributes, and time zones and server clocks that are off.
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
- Connecting through a jump host (ProxyJump): a connection can go through another saved
  connection, with its own key or password; the keys of both servers are checked, the connection
  is restored after a drop-out, and the terminal (Ctrl+P) goes the same way.
- Several ways to one server: a connection can have more than one address, for example one in the
  local network and one through a jump host. They are tried in the order you set, the local network
  first; when one does not work, the next is used. After a drop-out the program starts again from
  the first one, and it shows which way you are connected.
- SOCKS5 and HTTP CONNECT proxies and `ProxyCommand`, also when they come from `~/.ssh/config`.
- Server keys checked against `~/.ssh/known_hosts` too, next to the program's own file. The program
  never writes to `~/.ssh/known_hosts`: keys you accept go only to its own file. When the two files
  disagree about a server, the program does not connect and tells you why.
- Duplicating a connection in the site manager: a copy with all settings; you choose whether the
  saved password is copied too.
- Trash for local deleting.
- Custom keyboard shortcuts.
- Custom columns: which columns, in which order.
- A command line under the panels: on the left the current folder, then a field for a command. In
  a local panel the command runs in the current folder, in a server panel over SSH in the current
  remote folder. The output appears in a window. On a production server the command needs a
  confirmation first. A history of commands, never with passwords in it.
- A log of operations you can browse.
- Import of saved sites from WinSCP and FileZilla.

Already available: ssh-agent and passphrase-protected keys, colors by file type, an external editor,
and "Open with" associations per file type.

## 1.0 alpha and later: large features

- A Windows version from the same code, with the system-specific parts in separate modules.
- FTP and FTPS (explicit and implicit TLS) as another protocol: listings, transfers, resuming,
  reconnecting after a drop-out and the same protections of your data as with SFTP. Passwords only
  in the password vault; plain FTP without encryption only when you allow it in the connection
  settings.
- File sharing in the local network, compatible with the LocalSend protocol.
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
