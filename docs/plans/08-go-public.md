# Going public: checklist (plan 08 task 19)

What has to be set up by hand, in order. Checked 2026-10-27: `UnitCircleInc/uclog` is **public**
and its `main` is `0970e82`, an ancestor of this repository's `main`. Pushing there is a
fast-forward that publishes everything at once, so CI is tried first in a private staging
repository (B). `uclog` is free on PyPI and TestPyPI; `@uclog/element` (npm) and the `unitcircle`
publisher (Marketplace, Open VSX) don't exist yet; `uclog.unitcircle.ca` has no DNS record.

Secrets are added to a repository under Settings → Secrets and variables → Actions.

## A. Accounts and credentials (start now: some take days)

- [ ] **Apple** (ready, in 1Password). Prepare the five secret values:
  - `MACOS_CERTIFICATE_P12`: `base64 -i DeveloperID.p12 | pbcopy`
  - `MACOS_CERTIFICATE_PASSWORD`: the .p12's password
  - `APPLE_API_KEY_P8`: the text of the `AuthKey_XXXX.p8` file
  - `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`: from App Store Connect → Users and Access →
    Integrations
- [ ] **Windows**: Azure Artifact Signing (in progress). Not needed to go public: the download
  page and the docs say the Windows programs are not signed yet.
- [ ] **PyPI**: on pypi.org, Account → Publishing → *Add a new pending publisher*: project
  `uclog`, owner `UnitCircleInc`, repository `uclog`, workflow `release.yml`, environment
  `pypi`. The same on test.pypi.org with environment `testpypi`.
- [ ] **npm**: create the organisation `uclog` (npmjs.com → Add organization, free for public
  packages), owned by a Unit Circle account with 2FA. The first publish of `@uclog/element`
  needs a token (`NPM_TOKEN`: a granular token, read and write, for the `@uclog` scope). After it,
  add a trusted publisher on the package's settings (GitHub Actions, `UnitCircleInc/uclog`,
  `release.yml`, environment `npm`) and the token can go.
- [ ] **VS Code Marketplace**: an Azure DevOps organisation, then marketplace.visualstudio.com/manage
  → create publisher `unitcircle` (the id in `web/apps/vscode/package.json`). A personal access
  token with scope *Marketplace → Manage*, all accessible organisations: secret `VSCE_PAT`.
- [ ] **Open VSX**: sign in at open-vsx.org with GitHub, link an Eclipse account and sign the
  publisher agreement, create a token: secret `OVSX_PAT`. Create the namespace once:
  `npx ovsx create-namespace unitcircle -p <token>`.
- [ ] **Device mirror**: create `UnitCircleInc/uclog-device`, public and empty (no README).
  Make a deploy key:
  `ssh-keygen -t ed25519 -C uclog-device-mirror -N '' -f uclog-device-deploy`. Add
  `uclog-device-deploy.pub` to `uclog-device` (Settings → Deploy keys, **allow write
  access**); the private file becomes secret `UCLOG_DEVICE_DEPLOY_KEY` in `uclog` (step C3).
- [ ] **DNS**: in the `unitcircle.ca` zone, `uclog CNAME unitcircleinc.github.io.`
- [ ] **Domain verification** (prevents takeover): GitHub organisation settings → Pages →
  Add a domain `uclog.unitcircle.ca`, then the TXT record it shows.

## B. Staging: CI and a release dry run, in private

- [ ] Create `UnitCircleInc/uclog-staging`, **private**, empty.
- [ ] `git remote add staging git@github.com:UnitCircleInc/uclog-staging.git`, then
  `git push staging main`.
- [ ] Actions: `native`, `python`, `spec`, `web` and `device` run on the push. Send me any
  failure's log. (`pages` builds but doesn't deploy, and `device-mirror` is skipped: both only run
  in `UnitCircleInc/uclog`.)
- [ ] Add the five Apple secrets to `uclog-staging`.
- [ ] Actions → release → *Run workflow* on `main`. It builds every target, signs and notarises
  the macOS files (named `uclog-main-…`) and builds the wheels, VSIX and site parts as artifacts;
  the release job is skipped (not a tag). Check that every runner exists for the repository:
  `macos-13` (Intel), `ubuntu-22.04-arm`, `windows-11-arm` (the Arm ones may not be offered to a
  private repository; I can switch those rows to cross builds).
- [ ] Optional, to try the release job: `git tag v4-alpha1 && git push staging v4-alpha1`. It
  makes a draft pre-release in `uclog-staging`; the publish jobs only run in `UnitCircleInc/uclog`.
  Delete the draft and the tag afterwards.

## C. Publish

- [ ] C1. Versions: tell me to set 4 (`scripts/version.py set 4` and `set 4 --device`). I commit
  it; the tree must then match the tags `v4` and `device-v4`.
- [ ] C2. `git remote add origin git@github.com:UnitCircleInc/uclog.git`, then
  `git push --dry-run origin main` (it must be a fast-forward from `0970e82`), then
  `git push origin main`. The old `v1`–`v3` tags are already there.
- [ ] C3. Secrets in `uclog`: the five Apple ones and `UCLOG_DEVICE_DEPLOY_KEY`.
- [ ] C4. Environments in `uclog` (Settings → Environments): `testpypi`, `pypi`, `npm` and
  `vscode`, each with you as required reviewer, so nothing is published without your approval.
  Put `NPM_TOKEN` in `npm`'s secrets and `VSCE_PAT`, `OVSX_PAT` in `vscode`'s (or in the
  repository's).
- [ ] C5. Pages: Settings → Pages → Source *GitHub Actions*; custom domain `uclog.unitcircle.ca`;
  when the certificate is issued, *Enforce HTTPS*. Then Actions → pages → *Run workflow*.
  https://uclog.unitcircle.ca/ shows the docs from `main` at `/dev/` (and the app at `/app/`).
- [ ] C6. The mirror: the push in C2 runs `device-mirror` (it needs the deploy key from C3: if it
  ran first, re-run it). `uclog-device` gets `main` and `v1`–`v3`.
- [ ] C7. Repository settings: description, website `https://uclog.unitcircle.ca`, topics;
  protect `main` (no force push).
- [ ] C8. Release: `git tag v4 && git tag device-v4 && git push origin v4 device-v4`. `release.yml`
  makes the draft release `v4`, `device-mirror` the draft `device-v4` and the mirror's tag `v4`.
- [ ] C9. Check the drafts (the files, the notes, `uclog-release-4.json` says macOS signed), then
  publish `v4` and `device-v4`. Publishing runs `pages` again: `/v4/`, `/latest/` and
  `/download/` appear. In the release run, approve the waiting jobs: `testpypi` (then
  `testpypi-install` checks an install on Linux, macOS and Windows), `pypi`, `npm` and `vscode`.
  They don't wait for the draft to be published, so check the draft first. A pre-release tag
  publishes to TestPyPI, PyPI and npm (`next`), not to VS Code.
- [ ] C10. Check as a user: the download page on a Mac and on Windows; the `.dmg` on another Mac;
  `pip install uclog`; the VS Code extension from the Marketplace; a Zephyr app with
  `uclog-device` in its `west.yml`.
- [ ] C11. Announce from www.unitcircle.ca.

Existing users of the device repository keep working: `v1`–`v3` stay, and `main` keeps a
`zephyr/module.yml` at the root that points into `device/`.

## What I did meanwhile (no setup needed)

- The publish jobs in `release.yml` (plan 07 tasks 12, 13a and 13b), with the secret and
  environment names above.
- The smaller plan 07 items: 8 (Linux armv7: a `linux-armv7` archive, wheel and VSIX target), 9
  (the Linux access hint), 28 (`uclog hub --config`), 29 (GUI terminal character mode) and 30
  (GUI fallback fonts). 33 was already done.
