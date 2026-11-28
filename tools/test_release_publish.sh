#!/usr/bin/env bash
set -euo pipefail

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/assets"
touch "$tmp/assets"/{sidenote-0.1.1-dev.2.main.135-latest-full.nupkg,sidenote-latest-Setup.exe,SHA256SUMS,releases.latest.json}
export RUNNER_TEMP="$tmp" ASSET_DIR="$tmp/assets" DRY_RUN=1
export RELEASE_TAG=v0.1.1-dev.2.main.135 RELEASE_SHA=0123456789abcdef0123456789abcdef01234567
export GITHUB_EVENT_NAME=workflow_run GITHUB_REPOSITORY=fixture/sidenote

respond() {
    if [[ $1 == error ]]; then
        echo 'Simulated GitHub API error' >&2
        return 1
    fi
    printf '%s\n' "$1"
}

gh() {
    case "$1 $2" in
        "api repos/fixture/sidenote/compare/$RELEASE_SHA...heads/main?per_page=1") respond "$MAIN_STATUS" ;;
        "api repos/fixture/sidenote/git/matching-refs/tags/latest-main") respond "$TAG_EXISTS" ;;
        "api repos/fixture/sidenote/compare/tags/latest-main...$RELEASE_SHA?per_page=1") respond "$LATEST_STATUS" ;;
        'release view')
            if [[ $RELEASE_STATE == missing ]]; then
                echo 'release not found' >&2
                return 1
            fi
            case "$*" in
                *'--json isDraft,name'*) echo 'false Sidenote latest v0.1.1-dev.2.main.136' ;;
                *'--json assets'*) ;;
                *) echo "Unexpected release lookup: $*" >&2; return 1 ;;
            esac
            ;;
        *) echo "Unexpected GitHub call: $*" >&2; return 1 ;;
    esac
}
export -f gh respond

run_case() {
    local name=$1 expected_exit=$2 expected_output=$3
    local status=0 output
    shift 3
    output=$(MAIN_STATUS=$1 TAG_EXISTS=$2 LATEST_STATUS=$3 RELEASE_STATE=$4 \
        bash "$(dirname "$0")/release_publish.sh" 2>&1) || status=$?
    if [[ $status != "$expected_exit" || $output != *"$expected_output"* ]]; then
        printf '%s failed (exit %s):\n%s\n' "$name" "$status" "$output" >&2
        exit 1
    fi
    if [[ $expected_output == 'Would run:'* ]]; then
        if [[ $output != *'Would run: gh release edit latest-main'* ||
            $output == *'Would run: gh api -X DELETE'* ]]; then
            printf '%s did not finish the planned publish safely:\n%s\n' "$name" "$output" >&2
            exit 1
        fi
    elif [[ $output == *'Would run:'* ]]; then
        printf '%s planned an unexpected release write:\n%s\n' "$name" "$output" >&2
        exit 1
    fi
    echo "Passed: $name"
}

run_case 'main advanced after the candidate' 0 'Would run: gh api -X PATCH' ahead true ahead exists
run_case 'candidate matches Latest' 0 'identical relative to latest-main' ahead true identical exists
run_case 'candidate is older than Latest' 0 'behind relative to latest-main' ahead true behind exists
run_case 'candidate diverged from Latest' 0 'diverged relative to latest-main' ahead true diverged exists
run_case 'candidate is outside main history' 0 'outside main history' diverged false error missing
run_case 'candidate is ahead of main' 0 'outside main history' behind false error missing
run_case 'first Latest publish at main head' 0 'Would run: gh release create latest-main' identical false error missing
run_case 'tag exists without a release' 0 'Would run: gh api -X PATCH' ahead true ahead missing
run_case 'release exists without a tag' 0 'Would run: gh api -X POST' ahead false error exists
run_case 'main compare API failed' 1 'Failed to compare' error false error missing
run_case 'Latest compare API failed' 1 'Failed to compare' ahead true error exists
run_case 'Latest tag lookup failed' 1 'Failed to look up the latest-main tag' ahead error error exists
