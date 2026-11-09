# A test box for Remote

To try [a computer with no screen](guide/headless-host.md) for real, with your phone, you need a
Linux server reached over SSH. `scripts/test-box.sh` makes one on Hetzner Cloud, sets it up, builds
your checkout of Yardsort on it, and deletes it again when you are done, cleaning up after itself.
The maintainers use it for each Remote slice's run on a real box. It is for development: to run a
host you keep, follow [the guide](guide/headless-host.md) on a machine of your own.

Any Linux server works if you would rather use another provider: what the script does is written
out below, and [`scripts/test-box/cloud-init.yaml`](../scripts/test-box/cloud-init.yaml) is the
first boot, which most providers accept as it is.

## What you need

- A Hetzner account, and in its Cloud Console a project of its own for this, so the box never lands
  among anything else. Then **Security → API tokens → Generate API token**, with **Read & Write**.
- [`hcloud`](https://github.com/hetznercloud/cli), with the token kept in a context of the same name
  as the project. Type the token at its prompt yourself, never on a command line or in a chat:

  ```sh
  hcloud context create yardsort
  ```

- `gh`, signed in to your GitHub account, and `jq`, `rsync` and `ssh`.
- An SSH key, `~/.ssh/id_ed25519.pub` unless you set `TEST_BOX_KEY`.
- A Yardsort server to sign the host in to: your own, as
  [self-hosting](../crates/server/self-host/README.md) says, or one you have an account on.

## Using it

```sh
scripts/test-box.sh up       # about ten minutes
scripts/test-box.sh ssh      # log in as the box's yardsort user
scripts/test-box.sh sync     # after a change here: copy it over and build again
scripts/test-box.sh status   # whether it exists, and what yardsort-host says
scripts/test-box.sh down     # sign it out everywhere, then delete it
```

`up` makes a server called `yardsort-box`, a CX33 (4 vCPU, 8 GB) running Ubuntu 24.04, behind a
firewall that lets in SSH only, and:

- on its first boot: updates, the build tools, an ordinary `yardsort` user with your key, password
  logins off, a firewall on the box too, its programs kept running after you log out, and Rust;
- installs `gh` and Claude Code there;
- makes the box an SSH key and adds it to your GitHub account, and sets its git name and email from
  your GitHub profile;
- makes a private repository, `<you>/yardsort-box-demo`, unless it exists, and clones it to
  `~/src/demo` for the agents to work in;
- copies this checkout to `~/src/yardsort` and builds `yardsort-host` and `ys` there.

It prints what is left, since each is a sign-in of yours: `claude`, `gh auth login`,
`yardsort-host sign-in <server>`, then `yardsort-host serve` under `tmux`, `ys project add ~/src/demo`
and `yardsort-host pair` for the phone, as [the guide](guide/headless-host.md) says.

`sync` copies the checkout again and rebuilds. A `serve` already running keeps the old build until
you restart it, and the agents keep running either way.

`TEST_BOX_SERVER` sets the server those printed steps sign in to. `TEST_BOX_NAME`, `TEST_BOX_TYPE`,
`TEST_BOX_LOCATION`, `TEST_BOX_CONTEXT`, `TEST_BOX_REPO`, `TEST_BOX_GIT_NAME` and `TEST_BOX_GIT_EMAIL`
change the other defaults; the script's header lists them.

## What it costs, and when to take it down

Hetzner bills a server for as long as it exists, **powered off or not**. Only `down` stops the bill.
On 2026-11-07 a CX33 was €0.0136 an hour, at most €8.49 a month, plus its IPv4 address; `up` prints
the price when it creates the box. Keep it while you are working on something that needs it, and take
it down when that is done: `up` builds it again in about fifteen minutes.

## What it holds

**The box holds your sign-ins**: Claude Code's, `gh`'s, the host's, and an SSH key that can push to
every repository your GitHub account can. Anything running there as `yardsort`, every agent
included, can use them. So:

- Use it for throwaway work, like the demo repository.
- Do not snapshot it or turn on backups: either would keep the sign-ins.
- Take it down with `down`, not from the Console. `down` lists work there that is not pushed and
  asks first, then signs the host out at the server (or your phone keeps a computer that never comes
  back), signs `gh` out, removes the box's key from your GitHub account, and deletes the server. The
  demo repository stays; `gh repo delete` removes it.
