# Going public: checklist (plan 08 task 19)

What has to be set up by hand, in order. Checked 2026-10-27: `UnitCircleInc/uclog` is **public**
and its `main` is `0970e82`, an ancestor of this repository's `main`. Pushing there is a
fast-forward that publishes everything at once, so CI is tried first in a private staging
repository (B). `uclog` is free on PyPI and TestPyPI; `@uclog/element` (npm) and the `unitcircle`
publisher (Marketplace, Open VSX) don't exist yet; `uclog.unitcircle.ca` has no DNS record.

Secrets are added to a repository under Settings → Secrets and variables → Actions.

## A. Accounts and credentials (start now: some take days) — done 2026-10-29 except Windows

- [x] **Apple** (ready, in 1Password). Prepare the five secret values:
  - `MACOS_CERTIFICATE_P12`: `base64 -i DeveloperID.p12 | pbcopy`
  - `MACOS_CERTIFICATE_PASSWORD`: the .p12's password
  - `APPLE_API_KEY_P8`: the text of the `AuthKey_XXXX.p8` file
  - `APPLE_API_KEY_ID`, `APPLE_API_ISSUER_ID`: from App Store Connect → Users and Access →
    Integrations
- [ ] **Windows**: Azure Artifact Signing (in progress). Not needed to go public: the download
  page and the docs say the Windows programs are not signed yet.
- [x] **PyPI**: on pypi.org, Account → Publishing → *Add a new pending publisher*: project
  `uclog`, owner `UnitCircleInc`, repository `uclog`, workflow `release.yml`, environment
  `pypi`. The same on test.pypi.org with environment `testpypi`.
- [x] **npm**: create the organisation `uclog` (npmjs.com → Add organization, free for public
  packages), owned by a Unit Circle account with 2FA. No token: the first publish of
  `@uclog/element` is by hand (step C8), and CI publishes later versions by trusted publishing.
- [x] **VS Code Marketplace**: an Azure DevOps organisation, then marketplace.visualstudio.com/manage
  → create publisher `unitcircle` (the id in `web/apps/vscode/package.json`). No token: on the
  publisher's *Trusted Publishing* tab, add GitHub: `UnitCircleInc` / `uclog`, workflow
  `release.yml` (and environment `vscode`, if it asks). The release job runs
  `vsce publish --oidc`.
- [x] **Open VSX**: sign in at open-vsx.org with GitHub, link an Eclipse account and sign the
  publisher agreement. Create the namespace `unitcircle` (on the site, or with a token:
  `npx ovsx create-namespace unitcircle -p <token>`). Trusted publishing can only be added once
  the extension has a version, so the first publish is by hand (step C10).
- [x] **Device mirror**: create `UnitCircleInc/uclog-device`, public and empty (no README).
  Make a deploy key:
  `ssh-keygen -t ed25519 -C uclog-device-mirror -N '' -f uclog-device-deploy`. Add
  `uclog-device-deploy.pub` to `uclog-device` (Settings → Deploy keys, **allow write
  access**); the private file becomes secret `UCLOG_DEVICE_DEPLOY_KEY` in `uclog` (step C3).
- [x] **DNS**: in the `unitcircle.ca` zone, `uclog CNAME unitcircleinc.github.io.`
- [x] **Domain verification** (prevents takeover): GitHub organisation settings → Pages →
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
  None of them needs secrets.
- [ ] C5. Pages: Settings → Pages → Source *GitHub Actions*; custom domain `uclog.unitcircle.ca`;
  when the certificate is issued, *Enforce HTTPS*. Then Actions → pages → *Run workflow*.
  https://uclog.unitcircle.ca/ shows the docs from `main` at `/dev/` (and the app at `/app/`).
- [ ] C6. The mirror: the push in C2 runs `device-mirror` (it needs the deploy key from C3: if it
  ran first, re-run it). `uclog-device` gets `main` and `v1`–`v3`.
- [ ] C7. Repository settings: description, website `https://uclog.unitcircle.ca`, topics;
  protect `main` (no force push).
- [ ] C8. `@uclog/element` 4.0.0 to npm, by hand (trusted publishing can only be set up on a
  package that exists). From the tree of C1:
  ```
  npm login                     # in the browser, with your 2FA
  cd web
  pnpm install --frozen-lockfile
  pnpm --filter @uclog/element build
  cd packages/element
  pnpm pack                     # uclog-element-4.0.0.tgz; check its files: tar tzf
  npm publish uclog-element-4.0.0.tgz --access public   # asks for a 2FA code
  ```
  Then on npmjs.com, `@uclog/element` → Settings: *Trusted publisher* → GitHub Actions,
  organisation `UnitCircleInc`, repository `uclog`, workflow `release.yml`, environment `npm`.
  Under *Publishing access*, require 2FA and disallow tokens (trusted publishing still works).
- [ ] C9. Release: `git tag v4 && git tag device-v4 && git push origin v4 device-v4`. `release.yml`
  makes the draft release `v4`, `device-mirror` the draft `device-v4` and the mirror's tag `v4`.
- [ ] C10. Check the drafts (the files, the notes, `uclog-release-4.json` says macOS signed), then
  publish `v4` and `device-v4`. Publishing runs `pages` again: `/v4/`, `/latest/` and
  `/download/` appear. In the release run, approve the waiting jobs: `testpypi` (then
  `testpypi-install` checks an install on Linux, macOS and Windows), `pypi` and `vscode`. Not
  `npm` for `v4`: 4.0.0 is already there (C8), so reject it. From `v5` it publishes.
  Before approving `vscode` the first time, publish to Open VSX by hand: download
  `uclog-4.0.0.vsix` from the draft release, then
  `npx -y ovsx@1.2.0 publish uclog-4.0.0.vsix -p <token>` (a token from open-vsx.org → Settings →
  Access Tokens; delete it afterwards). Then Settings → Trusted Publishers: namespace
  `unitcircle`, `UnitCircleInc` / `uclog`, workflow `release.yml`, environment `vscode`. The
  job then publishes to the Marketplace and skips Open VSX's 4.0.0.
  They don't wait for the draft to be published, so check the draft first. A pre-release tag
  publishes to TestPyPI, PyPI and npm (`next`), not to VS Code.
- [ ] C11. Check as a user: the download page on a Mac and on Windows; the `.dmg` on another Mac;
  `pip install uclog`; the VS Code extension from the Marketplace; a Zephyr app with
  `uclog-device` in its `west.yml`.
- [ ] C12. Announce from www.unitcircle.ca.

Existing users of the device repository keep working: `v1`–`v3` stay, and `main` keeps a
`zephyr/module.yml` at the root that points into `device/`.

## What I did meanwhile (no setup needed)

- The publish jobs in `release.yml` (plan 07 tasks 12, 13a and 13b), with the secret and
  environment names above.
- The smaller plan 07 items: 8 (Linux armv7: a `linux-armv7` archive, wheel and VSIX target), 9
  (the Linux access hint), 28 (`uclog hub --config`), 29 (GUI terminal character mode) and 30
  (GUI fallback fonts). 33 was already done.
