#!/usr/bin/env bash
set -euo pipefail

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/assets"
touch "$tmp/assets"/{sidenote-0.1.1-dev.2.main.135-latest-full.nupkg,sidenote-latest-Setup.exe,releases.latest.json}
(cd "$tmp/assets" && sha256sum -- * > SHA256SUMS)
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
        'api repos/fixture/sidenote/git/matching-refs/tags/latest-main')
            case $TAG_RESPONSE in
                exact) echo '[{"ref":"refs/tags/latest-main"}]' ;;
                prefix) echo '[{"ref":"refs/tags/latest-main-old"}]' ;;
                absent) echo '[]' ;;
                *) respond "$TAG_RESPONSE" ;;
            esac
            ;;
        "api repos/fixture/sidenote/compare/tags/latest-main...$RELEASE_SHA?per_page=1") respond "$LATEST_STATUS" ;;
        'release download')
            if [[ $PUBLISHED_VERSION == error ]]; then respond error; return; fi
            printf '{"Assets":[{"Type":"Full","PackageId":"sidenote","Version":"%s","FileName":"sidenote-%s-latest-full.nupkg"}]}\n' \
                "$PUBLISHED_VERSION" "$PUBLISHED_VERSION"
            ;;
        'release view')
            if [[ $RELEASE_STATE == missing ]]; then
                echo 'release not found' >&2
                return 1
            fi
            case "$*" in
                *'--json isDraft,name'*)
                    case $RELEASE_STATE in
                        draft|bad-digest) echo "true Sidenote latest $RELEASE_TAG" ;;
                        complete) echo "false Sidenote latest $RELEASE_TAG" ;;
                        *) echo 'false Sidenote latest v0.1.1-dev.2.main.136' ;;
                    esac
                    ;;
                *'--json isPrerelease,body'*) cat "$RUNNER_TEMP/sidenote-$RELEASE_TAG-notes.md" ;;
                *'--json assets'*)
                    case $RELEASE_STATE in
                        draft|complete|incomplete|bad-digest)
                            local asset digest
                            for asset in "$ASSET_DIR"/*; do
                                digest=$(sha256sum -- "$asset")
                                if [[ $RELEASE_STATE == bad-digest && $asset == *.nupkg ]]; then digest=wrong; fi
                                printf '%s\tsha256:%s\n' "${asset##*/}" "${digest%% *}"
                            done
                            ;;
                    esac
                    ;;
                *) echo "Unexpected release lookup: $*" >&2; return 1 ;;
            esac
            ;;
        *) echo "Unexpected GitHub call: $*" >&2; return 1 ;;
    esac
}
export -f gh respond

run_case() {
    local name=$1 expected_exit=$2 expected_output=$3 expected_writes=$4
    local status=0 output writes
    shift 4
    output=$(MAIN_STATUS=$1 TAG_RESPONSE=$2 LATEST_STATUS=$3 RELEASE_STATE=$4 PUBLISHED_VERSION=${5:-0.1.1-dev.2.main.122} \
        bash "$(dirname "$0")/release_publish.sh" 2>&1) || status=$?
    writes=$(awk '/^Would run:/ {
        if ($4 == "release") print $5
        else if ($4 == "api" && $5 == "-X") print $6
        else print $0
    }' <<< "$output" | paste -sd ',' -)
    if [[ $status != "$expected_exit" || $output != *"$expected_output"* || $writes != "$expected_writes" ]]; then
        printf '%s failed (exit %s, writes %s):\n%s\n' "$name" "$status" "$writes" "$output" >&2
        exit 1
    fi
    echo "Passed: $name"
}

run_case 'main advanced after the candidate' 0 'Would run:' upload,upload,upload,PATCH,edit ahead exact ahead exists
run_case 'published Dev version is higher despite a lower main number' 0 'older than published Latest' '' ahead exact ahead exists 0.1.1-dev.3.main.1
run_case 'published core version is higher despite a lower Dev number' 0 'older than published Latest' '' ahead exact ahead exists 0.1.2-dev.0.main.1
run_case 'identical complete release skips' 0 'Latest already publishes the proved package' '' ahead exact identical complete
run_case 'identical interrupted draft finishes' 0 '--draft=false' edit ahead exact identical draft
run_case 'identical public release with stale metadata finishes' 0 '--draft=false' edit ahead exact identical incomplete
run_case 'equal public package with stale tag finishes' 0 '--draft=false' PATCH,edit ahead exact ahead incomplete 0.1.1-dev.2.main.135
run_case 'identical draft with mismatched package fails' 1 'differs from the proved package' '' ahead exact identical bad-digest
run_case 'identical tag without a release fails' 1 'its release is missing' '' ahead exact identical missing
run_case 'candidate is older than Latest' 0 'behind relative to latest-main' '' ahead exact behind exists
run_case 'candidate diverged from Latest' 0 'diverged relative to latest-main' '' ahead exact diverged exists
run_case 'candidate is outside main history' 0 'outside main history' '' diverged absent error missing
run_case 'candidate is ahead of main' 0 'outside main history' '' behind absent error missing
run_case 'first Latest publish at main head' 0 'Would run: gh release create latest-main' create,upload,upload,upload,edit identical prefix error missing
run_case 'tag exists without a release' 0 'Would run: gh release create latest-main' create,upload,upload,upload,PATCH,edit ahead exact ahead missing
run_case 'release exists without a tag' 0 'Would run:' upload,upload,upload,POST,edit ahead absent error exists
run_case 'main compare API failed' 1 'Failed to compare' '' error absent error missing
run_case 'Latest compare API failed' 1 'Failed to compare' '' ahead exact error exists
run_case 'Latest tag lookup failed' 1 'Failed to look up the latest-main tag' '' ahead error error exists
run_case 'matching-refs response is an object' 1 'Malformed latest-main tag lookup response' '' ahead '{"ref":"refs/tags/latest-main"}' error exists
run_case 'matching-refs entry is a string' 1 'Malformed latest-main tag lookup response' '' ahead '["refs/tags/latest-main"]' error exists
run_case 'matching-refs entry lacks ref' 1 'Malformed latest-main tag lookup response' '' ahead '[{}]' error exists
run_case 'matching-refs entry has a non-string ref' 1 'Malformed latest-main tag lookup response' '' ahead '[{"ref":7}]' error exists
run_case 'published feed download failed' 1 'Failed to download the published Latest feed' '' ahead exact ahead exists error
