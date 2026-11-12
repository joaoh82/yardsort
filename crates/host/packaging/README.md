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

`yardsort-host install-agent claude` (or `cursor`, `grok`, `codex`, `opencode`, `pi`, `omp`) installs an
agent into `~/.local`, as you, from its vendor's own installer or from npm.
