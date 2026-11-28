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

asset_inventory() {
    local asset digest
    for asset; do
        digest=$(sha256sum -- "$asset") || return 1
        if [[ $RELEASE_STATE == bad-digest && $asset == *.nupkg ]]; then
            digest=$(printf '%064d' 0)
        fi
        jq -cn --arg name "${asset##*/}" --arg digest "sha256:${digest%% *}" '{name: $name, digest: $digest}' || return 1
    done | jq -s '{assets: .}'
}

mock_upload() {
    local asset name replace=${*: -1}
    while [[ $1 != --repo ]]; do
        asset=$1
        name=${asset##*/}
        printf 'upload %s\n' "$name" >> "$WRITE_LOG"
        if [[ -e $REMOTE_ASSETS/$name ]]; then
            [[ $replace == --clobber ]] || return 1
            rm -- "$REMOTE_ASSETS/$name" || return 1
            if [[ $name == releases.latest.json && ${FAIL_FEED_UPLOAD:-0} == 1 ]]; then
                echo 'Simulated failed feed upload after deletion' >&2
                return 1
            fi
        fi
        cp -- "$asset" "$REMOTE_ASSETS/" || return 1
        shift
    done
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
            if [[ -n ${REMOTE_ASSETS:-} ]]; then cat "$REMOTE_ASSETS/releases.latest.json"; return; fi
            case $PUBLISHED_VERSION in
                error) respond error; return ;;
                malformed) echo '{"Assets":[]}'; return ;;
            esac
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
                    if [[ -n ${REMOTE_ASSETS:-} ]]; then
                        asset_inventory "$REMOTE_ASSETS"/*
                    else
                        case $RELEASE_STATE in
                            draft|complete|incomplete|bad-digest) asset_inventory "$ASSET_DIR"/* ;;
                            exists) asset_inventory "$ASSET_DIR/releases.latest.json" ;;
                            inventory-error) respond error ;;
                            inventory-empty) echo '{"assets":[]}' ;;
                            inventory-no-full)
                                jq -cn --arg digest "sha256:$(printf '%064d' 0)" \
                                    '{assets: [
                                        {name: "sidenote-0.1.1-dev.2.main.0135-latest-full.nupkg", digest: $digest},
                                        {name: "sidenote-0.1.1-dev.2.main.135-latest-delta.nupkg", digest: $digest}
                                    ]}'
                                ;;
                            inventory-malformed) echo '{"assets":[{}]}' ;;
                            inventory-no-response) : ;;
                            inventory-duplicate) asset_inventory "$ASSET_DIR/releases.latest.json" "$ASSET_DIR/releases.latest.json" ;;
                        esac
                    fi
                    ;;
                *) echo "Unexpected release lookup: $*" >&2; return 1 ;;
            esac
            ;;
        'release upload') [[ -n ${REMOTE_ASSETS:-} ]] && mock_upload "${@:4}" ;;
        'api -X') [[ -n ${REMOTE_ASSETS:-} ]] && printf '%s\n' "$3" >> "$WRITE_LOG" ;;
        'release edit') [[ -n ${REMOTE_ASSETS:-} ]] && echo edit >> "$WRITE_LOG" ;;
        *) echo "Unexpected GitHub call: $*" >&2; return 1 ;;
    esac
}
export -f gh respond asset_inventory mock_upload

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

run_case 'published feed cannot be parsed' 1 'Could not identify the published Latest package version' '' ahead exact ahead exists malformed
run_case 'published feed download failed for an identical tag' 1 'Failed to download the published Latest feed' '' ahead exact identical complete error
run_case 'Latest inventory API failed' 1 'Failed to list Latest assets' '' ahead exact ahead inventory-error
run_case 'Latest inventory has no response' 1 'Malformed Latest asset inventory' '' ahead exact ahead inventory-no-response
run_case 'Latest inventory entry lacks asset fields' 1 'Malformed Latest asset inventory' '' ahead exact ahead inventory-malformed
run_case 'Latest inventory has duplicate asset names' 1 'Malformed Latest asset inventory' '' ahead exact ahead inventory-duplicate
run_case 'absent feed without a retained full package' 1 'No usable retained Latest full package' '' ahead exact ahead inventory-empty
run_case 'absent feed with only a malformed full package and a delta' 1 'No usable retained Latest full package' '' ahead exact ahead inventory-no-full

prepare_artifact() {
    local dir=$1 version=${2#v}
    mkdir "$dir"
    printf 'full package %s\n' "$version" > "$dir/sidenote-$version-latest-full.nupkg"
    printf 'installer %s\n' "$version" > "$dir/sidenote-latest-Setup.exe"
    printf '{"Assets":[{"Type":"Full","PackageId":"sidenote","Version":"%s","FileName":"sidenote-%s-latest-full.nupkg"}]}\n' \
        "$version" "$version" > "$dir/releases.latest.json"
    (cd "$dir" && sha256sum -- * > SHA256SUMS)
}

run_recovery_case() {
    local name=$1 candidate=$2 candidate_tag=$3 damage=$4
    local expected_exit=$5 expected_output=$6 expected_writes=$7
    local output status=0
    local remote="$tmp/remote-$damage" log="$tmp/writes-$damage"
    local full="sidenote-${RELEASE_TAG#v}-latest-full.nupkg"
    local rolling_uploads=$'upload sidenote-latest-Setup.exe\nupload SHA256SUMS\nupload releases.latest.json'

    mkdir "$remote"
    cp -- "$tmp/older"/* "$remote/"
    : > "$log"
    output=$(MAIN_STATUS=ahead TAG_RESPONSE=exact LATEST_STATUS=ahead RELEASE_STATE=exists DRY_RUN=0 \
        ASSET_DIR="$tmp/proved" REMOTE_ASSETS="$remote" WRITE_LOG="$log" FAIL_FEED_UPLOAD=1 \
        bash "$(dirname "$0")/release_publish.sh" 2>&1) || status=$?
    if [[ $status != 1 || $output != *'Simulated failed feed upload after deletion'* ||
        $(<"$log") != "upload $full"$'\n'"$rolling_uploads" ||
        -e $remote/releases.latest.json || ! -f $remote/$full ]]; then
        printf '%s did not reproduce the deleted feed (exit %s):\n%s\n' "$name" "$status" "$output" >&2
        exit 1
    fi

    case $damage in
        package) echo corrupted > "$remote/$full" ;;
        installer) echo corrupted > "$remote/sidenote-latest-Setup.exe" ;;
    esac
    : > "$log"
    status=0
    output=$(MAIN_STATUS=ahead TAG_RESPONSE=exact LATEST_STATUS=ahead RELEASE_STATE=exists DRY_RUN=0 \
        ASSET_DIR="$candidate" RELEASE_TAG="$candidate_tag" REMOTE_ASSETS="$remote" WRITE_LOG="$log" \
        bash "$(dirname "$0")/release_publish.sh" 2>&1) || status=$?
    if [[ $status != "$expected_exit" || $output != *"$expected_output"* || $(<"$log") != "$expected_writes" ]]; then
        printf '%s failed (exit %s):\n%s\n' "$name" "$status" "$output" >&2
        cat "$log" >&2
        exit 1
    fi
    if [[ -n $expected_writes ]]; then
        cmp -- "$candidate/releases.latest.json" "$remote/releases.latest.json"
    else
        [[ ! -e $remote/releases.latest.json ]]
    fi
    echo "Passed: $name"
}

older_tag=v0.1.1-dev.1.main.999
newer_tag=v0.1.1-dev.2.main.136
prepare_artifact "$tmp/older" "$older_tag"
prepare_artifact "$tmp/proved" "$RELEASE_TAG"
prepare_artifact "$tmp/newer" "$newer_tag"
run_recovery_case 'same candidate repairs the deleted feed' "$tmp/proved" "$RELEASE_TAG" equal \
    0 'Rolling latest-main release branch: exists' $'upload releases.latest.json\nPATCH\nedit'
run_recovery_case 'higher candidate recovers the deleted feed' "$tmp/newer" "$newer_tag" higher \
    0 'Rolling latest-main release branch: exists' \
    $'upload sidenote-0.1.1-dev.2.main.136-latest-full.nupkg\nupload sidenote-latest-Setup.exe\nupload SHA256SUMS\nupload releases.latest.json\nPATCH\nedit'
run_recovery_case 'lower candidate cannot repair the deleted feed' "$tmp/older" "$older_tag" lower \
    0 'older than published Latest v0.1.1-dev.2.main.135' ''
run_recovery_case 'mismatched retained full package cannot repair the feed' "$tmp/proved" "$RELEASE_TAG" package \
    1 'differs from the proved package' ''
run_recovery_case 'mismatched retained installer cannot repair the feed' "$tmp/proved" "$RELEASE_TAG" installer \
    1 'differs from the proved package' ''
