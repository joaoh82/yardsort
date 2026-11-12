# What the full yardsort-host image carries

`yardsort-host` and `ys` are Yardsort's, under the GPL-3.0-only; their source is
https://github.com/joaoh82/yardsort, at the tag of the image's version. The image is built from
`crates/host/Dockerfile` and `crates/host/Dockerfile.full` there.

Beside them, each under its own licence, unmodified:

| What                        | Version               | Licence                                                 | Where it comes from                   |
| --------------------------- | --------------------- | ------------------------------------------------------- | ------------------------------------- |
| Debian 13 (trixie) packages | as the base pins them | each package's, in `/usr/share/doc/<package>/copyright` | https://www.debian.org                |
| Node.js and npm             | 26.13.0               | MIT, with others in its `LICENSE`                       | https://nodejs.org                    |
| Bun                         | 1.4.4                 | MIT, with others                                        | https://bun.sh                        |
| Go                          | 1.27.3                | BSD-3-Clause                                            | https://go.dev                        |
| Rust, Cargo, rustup         | 1.98.1, rustup 1.29.2 | MIT or Apache-2.0                                       | https://www.rust-lang.org             |
| GitHub CLI                  | 2.103.3               | MIT                                                     | https://github.com/cli/cli            |
| Codex CLI                   | 0.166.0               | Apache-2.0                                              | https://github.com/openai/codex       |
| OpenCode                    | 2.2.2                 | MIT                                                     | https://github.com/anomalyco/opencode |
| Pi                          | 1.4.0                 | MIT                                                     | https://github.com/earendil-works/pi  |
| Oh My Pi                    | 18.14.0               | MIT                                                     | https://github.com/can1357/oh-my-pi   |

The four harnesses' dependencies are in `/opt/yardsort-agents/node_modules`, each with the licence
its package names and the licence file it ships. All are under licences that allow redistribution
(MIT, Apache-2.0, BSD, ISC, BlueOak, MPL-2.0, Unlicense, 0BSD, CC0). One is under the
LGPL-3.0-or-later: libvips, as `@img/sharp-libvips-linux-*`, a shared library used unmodified; its
source is https://github.com/libvips/libvips, and the build of it is https://github.com/lovell/sharp-libvips.

**Not in the image**, because their licences do not let anyone hand a copy on: Claude Code, Cursor's
CLI and Grok. `yardsort-host install-agent <harness>` runs each vendor's own installer, as you, so
that you accept that vendor's terms.
