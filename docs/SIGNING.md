# Release signing (maintainers)

The in-app updater (`src/update.rs`) installs release assets as root, so every
asset is signed with [minisign](https://jedisct1.github.io/minisign/)
(Ed25519). The updater refuses to install anything that is not signed with the
key embedded at build time from `data/release-signing.pub`.

## What the updater checks

For the asset that matches the install (AppImage, `.deb`, `.rpm`,
`.pkg.tar.zst`, `x86_64.tar.gz`):

1. The build has a real public key. While `data/release-signing.pub` still
   holds the placeholder `REPLACE_WITH_MINISIGN_PUBLIC_KEY`, in-app updates
   are refused (fail closed).
2. The release is newer than the running version. Older or equal versions are
   refused unless the user runs `legion-cli check-update --apply --allow-downgrade`.
3. `<asset>.minisig` is published next to the asset. Downloads use HTTPS only
   (`curl --proto =https --proto-redir =https --tlsv1.2`).
4. The signature is valid for the downloaded bytes, and its trusted comment is
   exactly `file:<asset name> version:<tag>` for this asset and release. This
   stops an attacker from serving a different, older, or renamed signed asset.
5. For root installs, the pkexec script copies the file into a fresh
   root-owned 0700 directory under `/run`, checks the sha256 that the
   unprivileged side computed over the verified bytes, and only then runs
   `apt-get`, `dnf`/`zypper`/`rpm`, `pacman -U`, or extracts the tarball.

Source-tree installs (`git pull` and rebuild) are not affected. They follow
the git remote.

## One-time setup

Install minisign (`pacman -S minisign`, `apt install minisign`, or
`dnf install minisign`). Then, from the repository root:

```sh
mkdir -p ~/.minisign && chmod 700 ~/.minisign
rm data/release-signing.pub          # minisign will not overwrite the placeholder
minisign -G -p data/release-signing.pub -s ~/.minisign/legion-control.key
```

minisign asks for a password, which encrypts the secret key. This is
recommended. If you want a key without a password, add `-W`. In that case,
do not set the `MINISIGN_PASSWORD` secret.

Commit the public key:

```sh
git add data/release-signing.pub
git commit -m "chore(release): add minisign release signing public key"
```

Add the two repository secrets used by the `release` job in
`.github/workflows/build.yml`:

```sh
gh secret set MINISIGN_SECRET_KEY < ~/.minisign/legion-control.key
gh secret set MINISIGN_PASSWORD     # paste the key password when prompted; skip for a -W key
```

The CI step writes the key to a 0600 temp file, signs each asset with
`minisign -S -t "file:<name> version:<tag>"` (password on stdin), verifies the
signature against `data/release-signing.pub`, and uploads the `.minisig` files
with the release. The release job fails without publishing anything in any of
these cases:

- `MINISIGN_SECRET_KEY` is missing.
- The public key file is still the placeholder.
- The secret key does not match the committed public key.
- No assets were found to sign.

### Check a release by hand

```sh
minisign -V -p data/release-signing.pub -m legion-control_X.Y.Z_amd64.deb
# prints: Trusted comment: file:legion-control_X.Y.Z_amd64.deb version:vX.Y.Z
```

## Back up the secret key

- Keep at least one offline copy of `~/.minisign/legion-control.key`, for
  example on an encrypted USB stick or as an attachment in your password
  manager. Store the password separately from the key file.
- GitHub secrets cannot be read back, so the GitHub copy is not a backup.
- If you lose the key, you cannot sign updates that existing installs will
  accept. Users would have to reinstall by hand (see rotation below).

## Rotating the key

The updater trusts exactly one key: the one compiled into the running build.

### Planned rotation (old key still safe)

1. Generate a new key pair (as above, with a new file name) and replace
   `data/release-signing.pub` with the new public key.
2. Tag the next release, but sign it with the old key. Keep the old secrets
   for this one release. To make the CI public-key check pass, temporarily
   keep the old public key in a separate file and point the `minisign -V` line
   at it, or sign that release by hand. Installed copies verify it with the
   old key they embed, and the release they install embeds the new key.
3. Update the secrets to the new key (`gh secret set ...` as above) for every
   later release.
4. Delete the old secret key once enough users are on the new build.

### Compromised key

You cannot do an in-band rotation, because an attacker holding the old key can
sign anything that old builds will accept. Do the following:

1. Generate a new key, commit the new public key, update both secrets, and
   publish a release.
2. Announce the compromise (README, release notes, issue tracker). Tell users
   to reinstall the new release manually from GitHub and to check it with
   `minisign -V` against the new public key.
3. Consider deleting releases that were published while the key was
   compromised.

## Bootstrapping note

Builds from before signed updates (v0.2.16 and earlier) do not check
signatures. They still update to the first signed release through their old
path. From then on, every in-app update needs a valid signature. Builds made
while `data/release-signing.pub` is still the placeholder refuse in-app
updates. The release job also refuses to publish them.
