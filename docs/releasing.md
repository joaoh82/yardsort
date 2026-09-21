# Releasing

A release is a **version tag**. Pushing `v0.2.0` makes GitHub Actions build installers for Linux,
macOS and Windows and publish them as a GitHub release — no further clicks.

While the builds run, the release exists only as a draft. It is made public by the last job, and
only if **every** platform built; if one fails, the draft stays hidden for you to inspect, fix and
re-run.

## Cutting a release

### First, the changelog

`just release` never reads `CHANGELOG.md`. Renaming `## Unreleased` to the version you are cutting
is a manual step — and it is where entries go astray.

A branch opened before that rename writes its bullets under `## Unreleased`. By the time it merges,
that heading has become a version, so git files the new bullet under a section that has **already
shipped**. Nothing conflicts and nothing warns. It has happened; check before tagging, not after.

```sh
git log --oneline v0.3.2..HEAD          # what this release actually contains
git diff v0.3.2 HEAD -- CHANGELOG.md    # bullets added under an already-released heading
```

The second command is the one that catches it: an added line inside a section at or below the last
tag belongs further up. Move it first. Once the tag is pushed the notes are wrong in two places at
once — the new release omits the entry, and a published one claims something it never contained.

```sh
just release 0.2.0
```

That recipe checks the tree is clean and on `main`, runs `just check`, sets the version in
`Cargo.toml`, `package.json` and `src-tauri/tauri.conf.json`, commits `Release v0.2.0`, tags it and
pushes both. Then:

1. Watch the run: `just ci-watch`, or the repository's _Actions_ tab. About 20 minutes.
2. The release appears under _Releases_ with notes generated from the commits since the last tag.
   Edit them afterwards if you like.

If a build fails: fix the cause, then re-run the failed jobs from the _Actions_ tab (or run the
_Release_ workflow by hand with the same tag). To abandon the attempt instead, delete the draft
and the tag: `gh release delete v0.2.0 --cleanup-tag`.

Versions follow [semver](https://semver.org). A tag with a suffix — `v0.2.0-beta.1` — is marked as
a pre-release automatically.

To rebuild without a new tag, run the _Release_ workflow by hand (_Actions → Release → Run
workflow_) and give it an existing tag.

## What gets built

| System                                   | Artifacts                                              | Signing                                                                                |
| ---------------------------------------- | ------------------------------------------------------ | -------------------------------------------------------------------------------------- |
| Linux (x86_64)                           | `.AppImage`, `.deb`, `.rpm`; `yardsort-bin` on the AUR | —                                                                                      |
| macOS (universal: Apple Silicon + Intel) | `.dmg`, `.app.tar.gz`                                  | Signed and notarized when the Apple secrets below are set; otherwise an unsigned build |
| Windows (x86_64)                         | `-setup.exe` (NSIS), `.msi`                            | Not signed yet — users see a SmartScreen warning                                       |

## macOS signing and notarization

Needs a paid Apple Developer account. Set these repository secrets
(_Settings → Secrets and variables → Actions_); the workflow picks them up with no other change:

| Secret                       | What it is                                                                                                                                    |
| ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `APPLE_CERTIFICATE`          | Your **Developer ID Application** certificate, exported from Keychain Access as a `.p12`, then base64-encoded: `base64 -i cert.p12 \| pbcopy` |
| `APPLE_CERTIFICATE_PASSWORD` | The password you gave the `.p12` export                                                                                                       |
| `APPLE_SIGNING_IDENTITY`     | e.g. `Developer ID Application: Your Name (TEAMID)` — `security find-identity -v -p codesigning` lists it                                     |
| `APPLE_ID`                   | The Apple ID email of the developer account                                                                                                   |
| `APPLE_PASSWORD`             | An **app-specific password** for that Apple ID, from [account.apple.com](https://account.apple.com) → _Sign-In and Security_                  |
| `APPLE_TEAM_ID`              | The ten-character team id, from the developer portal's _Membership_ page                                                                      |

Creating the certificate: in Xcode → _Settings → Accounts → Manage Certificates → + → Developer ID
Application_; or in the developer portal under _Certificates_. Then export it from Keychain Access
(select the certificate **with its private key** → _Export_).

With `gh`:

```sh
gh secret set APPLE_CERTIFICATE < <(base64 -i cert.p12)
gh secret set APPLE_CERTIFICATE_PASSWORD
gh secret set APPLE_SIGNING_IDENTITY --body "Developer ID Application: Your Name (TEAMID)"
gh secret set APPLE_ID --body "you@example.com"
gh secret set APPLE_PASSWORD
gh secret set APPLE_TEAM_ID --body "ABCDE12345"
```

Until they are set, macOS builds still succeed but are unsigned, and Gatekeeper will refuse them
unless the user removes the quarantine attribute (see
[troubleshooting](guide/troubleshooting.md#macos)).

## Arch Linux: the AUR package

Each full release (not pre-releases) is also published to the
[Arch User Repository](https://aur.archlinux.org/packages/yardsort-bin) as **`yardsort-bin`**, which
repackages the release's `.deb`. The last job of the workflow renders `packaging/aur/PKGBUILD.in`
with the version and checksums, **builds the package in an Arch container to prove it works**, and
pushes `PKGBUILD` and `.SRCINFO` to the AUR.

It needs one secret, `AUR_SSH_PRIVATE_KEY`: the private half of an SSH key whose public half is
registered on the maintainer's AUR account (_My Account → SSH Public Key_). Use a key made only
for this:

```sh
ssh-keygen -t ed25519 -N "" -C "yardsort release workflow" -f aur_deploy_key
gh secret set AUR_SSH_PRIVATE_KEY < aur_deploy_key
cat aur_deploy_key.pub          # paste this into your AUR account, then delete both files
```

Without the secret the job fails, saying which secret is missing. To publish by hand instead:

```sh
git clone ssh://aur@aur.archlinux.org/yardsort-bin.git /tmp/yardsort-bin
scripts/aur-render.sh 0.2.0 /tmp/yardsort-bin      # writes PKGBUILD and .SRCINFO
cd /tmp/yardsort-bin && makepkg -f && git add PKGBUILD .SRCINFO && git commit -m "Update to 0.2.0" && git push
```

The name is `yardsort-bin` because it ships a prebuilt binary; a build-from-source `yardsort`
package is welcome from anyone who wants to maintain one.

## Homebrew

macOS users can `brew install --cask joaoh82/yardsort/yardsort`. The cask lives in its own tap
repository, [joaoh82/homebrew-yardsort](https://github.com/joaoh82/homebrew-yardsort), and is
generated from `packaging/homebrew/yardsort.rb.in`.

The **Homebrew** job runs on macOS and proves the cask before publishing it: `brew style`,
`brew audit --strict --online`, a real `brew install`, then `codesign --verify`, Gatekeeper's
verdict (`spctl`, which must say _Notarized Developer ID_) and the installed version. Only then
does it push to the tap.

It needs `HOMEBREW_TAP_DEPLOY_KEY`: the private half of an SSH **deploy key** registered, with
write access, on the tap repository only — it can touch nothing else.

```sh
ssh-keygen -t ed25519 -N "" -C "yardsort release workflow (homebrew tap)" -f tap_key
gh repo deploy-key add tap_key.pub -R joaoh82/homebrew-yardsort --allow-write --title "yardsort release workflow"
gh secret set HOMEBREW_TAP_DEPLOY_KEY -R joaoh82/yardsort < tap_key
```

The cask declares `auto_updates true`, because the app updates itself; `brew upgrade` therefore
leaves it alone unless given `--greedy`.

## winget

Windows users can `winget install joaoh82.Yardsort` once Microsoft has accepted the package. Each
version is a pull request against [microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs)
adding three manifests under `manifests/j/joaoh82/Yardsort/<version>/`, generated from
`packaging/winget/*.in`. Microsoft's pipeline validates it (it installs the package in a sandbox)
and a moderator merges it — hours to days, longer for a first submission.

The **winget** job opens that pull request through the API (`scripts/winget-submit.sh`). It does
nothing while an earlier Yardsort pull request is still open. It needs `WINGET_TOKEN`: a
**classic** personal access token with the `public_repo` scope, from the account that owns the
`winget-pkgs` fork ([create one](https://github.com/settings/tokens/new?scopes=public_repo&description=yardsort%20winget)):

```sh
gh secret set WINGET_TOKEN -R joaoh82/yardsort      # paste the token
```

The first contribution from an account also needs Microsoft's CLA: the bot asks in the pull
request, and the answer is a comment saying `@microsoft-github-policy-service agree`.

## Publishing to package managers by hand

**A job that cannot publish fails.** Until a credential is in place, every release run ends red
on that job — `Package managers / AUR`, `/ winget` — with an error saying which secret is
missing and what to do. That is deliberate: these jobs used to warn and exit 0, on the grounds
that a key nobody has registered yet is a setup step rather than a broken release, and the
result was a green run that had published nothing. It misled a reader within a day of shipping.

**A red package job does not mean the release failed.** This workflow runs _after_ the release is
public and nothing waits on it, so the installers are out and the download links work. Fix the
credential and re-run the workflow with the same tag, below.

"Already at this version" is still a success: nothing to push is not the same as nothing being
able to push.

All three jobs live in `.github/workflows/packages.yml`, which the Release workflow calls once a
release is public. It can also be run alone — _Actions → Package managers → Run workflow_, or
`gh workflow run packages.yml -f tag=v0.3.1` — to publish or re-publish an existing release
without rebuilding anything: after adding a missing credential, say, or once winget has merged an
earlier version. (Re-running the _Release_ workflow itself is different: it rebuilds and re-uploads
the installers, which changes their checksums. Avoid it for a release that is already public.)

## Windows signing

Deliberately not set up: certificates cost money and the project has none. If that changes,
Tauri's [Windows signing guide](https://tauri.app/distribute/sign/windows/) applies and the
workflow needs the certificate passed to the build step.

## Updates: signing and `latest.json`

Installed copies of Yardsort find new versions by reading
`https://github.com/joaoh82/yardsort/releases/latest/download/latest.json`, and install one only
if its signature verifies against the public key in `src-tauri/tauri.conf.json`
(`plugins.updater.pubkey`).

The release workflow builds with `src-tauri/tauri.release.conf.json`, which turns on
`createUpdaterArtifacts`: the self-updating bundles (AppImage, the macOS `.app.tar.gz`, the NSIS
and MSI installers) each get a `.sig`, and `latest.json` lists them. The publish job **refuses to
make a release public if `latest.json` is missing a platform** — such a release would silently
strand everyone on older versions.

It needs two secrets: `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

> **This key is the project's most important secret.** If it is **lost**, no installed copy can
> ever auto-update again — users must reinstall by hand. If it **leaks**, anyone who can also
> publish a release here can ship code to every user. Keep an offline backup. To rotate it, ship a
> release signed with the _old_ key whose app carries the _new_ public key, then switch the secrets.

Local builds (`just build`) do not create updater artifacts and need no key. Which copies update
themselves is decided by how they were packaged (`src-tauri/src/updates.rs`): `.deb`, `.rpm` and
the AUR package are owned by a package manager and are only told that a new version exists.

To rehearse the whole flow against your own server, start a release build with
`YARDSORT_UPDATE_ENDPOINT=https://…/latest.json`.

## Not there yet

- **More package managers** — Flatpak, Scoop, Chocolatey.
