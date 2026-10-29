#!/usr/bin/env bash
# Run release.yml locally with act until the repository has a GitHub remote (plan 07 §2).
#
#   .github/act-release.sh [--x64] [act options...]
#
# The Linux jobs run in Docker. On Apple Silicon the Linux build row is linux-arm64; --x64 runs
# the containers under linux/amd64 emulation (slow) and builds linux-x64 instead. The macOS rows run
# directly on this Mac (act's -self-hosted), so the Mac needs rustup on PATH (direnv: .envrc) and
# the rust-toolchain step adds the x86_64-apple-darwin target to its stable toolchain. Windows rows
# cannot run under act. The wheels job skips the missing platforms; the release job is left out.
#
# act gives a matrix job the first row's platform, so each build row is its own act run. act's run
# id is always 1, so the runs share tmp/act-artifacts/1/. The macos, wheels and vsix jobs then run
# from a copy of the workflow without their `needs:` (act -j also runs a job's needs). The macos job
# has no secrets under act, so it passes the archives on unsigned
# (packaging/macos/release.sh signs locally).
#
# Docker's memory (8 GB by default, shared with any other running containers) is too small for the
# release profile's fat-LTO link of the uclog GUI, so the Linux build here uses thin LTO and 2 cargo
# jobs (ACT_LTO=fat, ACT_CARGO_JOBS=N to change). Its binaries are for checking the workflow, not
# for shipping.
set -euo pipefail
cd "$(dirname "$0")/.."

linux=linux-arm64 arch=(--container-architecture linux/arm64)
if [[ "${1:-}" == "--x64" ]]; then
  linux=linux-x64 arch=(--container-architecture linux/amd64); shift
fi

rm -rf tmp/act-artifacts tmp/act
mkdir -p tmp/act
sed -E -e '/^    needs: (build|\[build, macos(, windows)?\])$/d' -e '/^  release:/,$d' .github/workflows/release.yml > tmp/act/release.yml

run() {
  act workflow_dispatch \
    -P ubuntu-latest=catthehacker/ubuntu:act-22.04 \
    -P ubuntu-22.04=catthehacker/ubuntu:act-22.04 \
    -P ubuntu-22.04-arm=catthehacker/ubuntu:act-22.04 \
    -P macos-14=-self-hosted \
    --artifact-server-path tmp/act-artifacts "${arch[@]}" "$@"
}

run -W .github/workflows/release.yml -j build --matrix name:"$linux" --env CARGO_BUILD_JOBS="${ACT_CARGO_JOBS:-2}" --env CARGO_PROFILE_RELEASE_LTO="${ACT_LTO:-thin}" "$@"
for row in macos-arm64 macos-x64; do
  run -W .github/workflows/release.yml -j build --matrix name:"$row" "$@"
done
run -W tmp/act/release.yml -j macos "$@"
run -W tmp/act/release.yml -j wheels "$@"
run -W tmp/act/release.yml -j vsix "$@"
ls -R tmp/act-artifacts/1
