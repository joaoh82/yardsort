# yardsort-host

Yardsort on a Linux computer with no screen, as a host your phone reaches. The whole guide is at
https://yardsort.sh/docs/guide/headless-host.

What is here:

- `bin/yardsort-host`: the host. It is the terminal daemon too, so the two are always the same build.
- `bin/ys`: Yardsort's command line, to add projects and start agents.
- `systemd/yardsort-host.service`: a systemd user service that runs `yardsort-host serve`.
- `LICENSE`: the GNU General Public License, version 3.

Install for one user, sign in, then run it as a service:

```sh
install -Dm755 bin/yardsort-host bin/ys -t ~/.local/bin/
install -Dm644 systemd/yardsort-host.service -t ~/.config/systemd/user/
yardsort-host sign-in https://remote.example.com
systemctl --user daemon-reload
systemctl --user enable --now yardsort-host
sudo loginctl enable-linger "$USER"
yardsort-host pair
```

`~/.local/bin` must be on your `PATH`, in your login shell's profile: you and the agents find `ys` by it.

The guide's install line does the two `install` lines for you, from the newest release, and upgrades
the same way.

`yardsort-host install-agent claude` (or `cursor`, `grok`, `codex`, `opencode`, `pi`, `omp`) installs an
agent into `~/.local`, as you, from its vendor's own installer or from npm.

While it runs, the host serves a dashboard on `127.0.0.1:28680`, for doing the rest from a browser:
installing agents, Git and GitHub, setup commands, a terminal, signing in to a server and pairing. `yardsort-host dashboard` prints a one-time
link that signs a browser in; from another computer, forward the port over SSH first
(`ssh -L 28680:127.0.0.1:28680 <this computer>`). `yardsort-host dashboard off` turns it off. See
https://yardsort.sh/docs/guide/host-dashboard.
