#!/usr/bin/env bash
set -euo pipefail

[[ ${DRY_RUN:-} == 0 || ${DRY_RUN:-} == 1 ]] || { echo 'DRY_RUN must be exactly 0 or 1.' >&2; exit 1; }
event=${GITHUB_EVENT_NAME:-}
[[ $DRY_RUN != 0 || $event == push || $event == workflow_run ]] || { echo 'DRY_RUN=0 requires a tag push or a main CI completion.' >&2; exit 1; }

tag=${RELEASE_TAG:?Set RELEASE_TAG}
repo=${GITHUB_REPOSITORY:?Set GITHUB_REPOSITORY}
sha=${RELEASE_SHA:-${GITHUB_SHA:?Set GITHUB_SHA}}
asset_dir=${ASSET_DIR:?Set ASSET_DIR}
runner_temp=${RUNNER_TEMP:?Set RUNNER_TEMP}
stable='^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
dev='^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-dev\.(0|[1-9][0-9]*)$'
latest="${dev%$}\\.main\\.([1-9][0-9]*)\$"
if [[ $event == workflow_run && $tag =~ $latest ]]; then
    channel=latest
elif [[ $event == workflow_run ]]; then
    echo 'A main CI completion requires a Latest build identity.' >&2
    exit 1
elif [[ $tag =~ $stable ]]; then
    channel=stable
elif [[ $tag =~ $dev ]]; then
    channel=dev
else
    echo "Unexpected release tag: $tag" >&2
    exit 1
fi

notes_source="docs/releases/$tag.md"
notes_file="$runner_temp/sidenote-$tag-notes.md"
if [[ $channel == latest ]]; then
    printf 'Latest build %s from main commit %s.\n' "$tag" "$sha" > "$notes_file"
elif [[ -f $notes_source ]]; then
    cat "$notes_source" > "$notes_file"
elif [[ $event == push ]]; then
    echo "Missing required release notes: $notes_source" >&2
    exit 1
else
    echo "$notes_source is missing; using placeholder notes for this dry run."
    printf 'Placeholder release notes for %s.\n' "$tag" > "$notes_file"
fi
cat >>"$notes_file" <<'NOTES'

## Verify your download

Download `SHA256SUMS` from the same release, then run `Get-FileHash <file> -Algorithm SHA256` in PowerShell and compare the result with the matching line in `SHA256SUMS`.
NOTES
echo "Built release notes for $tag at $notes_file"

shopt -s nullglob
assets=("$asset_dir"/*)
if (( ${#assets[@]} == 0 )); then
    echo "No release files were downloaded to $asset_dir." >&2
    exit 1
fi

print_command() {
    printf 'Would run:'
    printf ' %q' "$@"
    printf '\n'
}

run_write() {
    if [[ $DRY_RUN == 0 ]]; then "$@"; else print_command "$@"; fi
}

version_cmp() {
    local left=$1 right=$2 count=$3 regex=$4 i left_part right_part LC_ALL=C
    [[ $left =~ $regex ]] || return 1
    local -a left_parts=("${BASH_REMATCH[@]:1:$count}")
    [[ $right =~ $regex ]] || return 1
    local -a right_parts=("${BASH_REMATCH[@]:1:$count}")
    for ((i = 0; i < count; i++)); do
        # Compare canonical decimal components without converting them to fixed-width integers.
        left_part=${left_parts[i]} right_part=${right_parts[i]}
        if (( ${#left_part} < ${#right_part} )); then printf '%s\n' -1; return 0; fi
        if (( ${#left_part} > ${#right_part} )); then printf '%s\n' 1; return 0; fi
        if [[ $left_part < $right_part ]]; then printf '%s\n' -1; return 0; fi
        if [[ $left_part > $right_part ]]; then printf '%s\n' 1; return 0; fi
    done
    printf '%s\n' 0
}

read_err="$runner_temp/release-publish-read.$$.err"
lookup_out="$runner_temp/release-publish-lookup.$$.out"

read_or_fail() {
    local message=$1
    shift
    if "$@" 2>"$read_err"; then return 0; fi
    cat "$read_err" >&2
    echo "$message" >&2
    return 1
}

lookup_or_missing() {
    local missing_re=$1 message=$2
    shift 2
    if "$@" >"$lookup_out" 2>"$read_err"; then
        cat "$lookup_out"
        return 0
    fi
    if grep -qiE "$missing_re" "$lookup_out" "$read_err"; then return 1; fi
    cat "$lookup_out" "$read_err" >&2
    echo "$message" >&2
    return 2
}

remote_tag_sha() {
    local object_info object_sha object_type
    object_info=$(read_or_fail "Failed to resolve remote tag $tag." gh api "repos/$repo/git/ref/tags/$tag" --jq '.object.sha + " " + .object.type') || return 1
    read -r object_sha object_type <<<"$object_info"
    if [[ $object_type == tag ]]; then
        object_sha=$(read_or_fail "Failed to dereference annotated tag $tag." gh api "repos/$repo/git/tags/$object_sha" --jq .object.sha) || return 1
    fi
    printf '%s\n' "$object_sha"
}

compare_status() {
    local base=$1 head=$2
    read_or_fail "Failed to compare $base with $head." \
        gh api "repos/$repo/compare/$base...$head?per_page=1" --jq .status
}

latest_inventory() {
    read_or_fail 'Failed to list Latest assets.' gh release view latest-main --repo "$repo" --json assets |
        jq -ers '
            def valid_asset:
                type == "object" and
                (.name | type == "string" and length > 0 and (test("[[:cntrl:]]") | not)) and
                (.digest | type == "string" and length == 71 and test("^sha256:[0-9a-f]{64}$"));
            def valid_inventory:
                type == "object" and (.assets |
                    type == "array" and all(.[]; valid_asset) and
                    (map(.name) | length == (unique | length)));
            if length == 1 and (.[0] | valid_inventory) then
                .[0].assets | map(.name + "\t" + .digest) | join("\n")
            else error("Malformed Latest asset inventory") end'
}

retained_latest_version() {
    local pattern=$1 highest= name version
    shift
    for name; do
        if [[ $name == *-full.nupkg && $name =~ $pattern ]]; then
            version=${BASH_REMATCH[1]}
            if [[ -z $highest ]] || [[ $(version_cmp "v$version" "v$highest" 5 "$latest") == 1 ]]; then
                highest=$version
            fi
        fi
    done
    [[ -n $highest ]] || { echo 'No usable retained Latest full package.' >&2; return 1; }
    printf '%s\n' "$highest"
}

published_latest_version() {
    read_or_fail 'Failed to download the published Latest feed.' gh release download latest-main \
        --repo "$repo" --pattern releases.latest.json --output - |
        jq -er '[.Assets | arrays[] | select(.Type == "Full")] | select(length == 1) | .[0] |
            select(.PackageId == "sidenote" and (.Version | type) == "string") |
            select(.FileName == ("sidenote-" + .Version + "-latest-full.nupkg")) | .Version' || {
        echo 'Could not identify the published Latest package version.' >&2
        return 1
    }
}

latest_asset_matches() {
    local asset=$1 checksum
    checksum=$(sha256sum -- "$asset") || return 1
    [[ ${existing[${asset##*/}]-} == "sha256:${checksum%% *}" ]]
}

finish_latest_release() {
    if [[ $latest_tag_exists == true && $latest_status != identical ]]; then
        run_write gh api -X PATCH "repos/$repo/git/refs/tags/latest-main" -f "sha=$sha" -F force=true --silent
    elif [[ $latest_tag_exists == false && $rolling_draft == false ]]; then
        run_write gh api -X POST "repos/$repo/git/refs" -f ref=refs/tags/latest-main -f "sha=$sha" --silent
    fi
    run_write gh release edit latest-main --repo "$repo" --target "$sha" --title "Sidenote latest $tag" \
        --notes-file "$notes_file" --draft=false --prerelease --latest=false
}

refresh_latest_release() {
    local asset name digest inventory version previous= count=0 published_version published_cmp= published_notes
    local feed_missing=false repair_feed=false feed="$asset_dir/releases.latest.json"
    local -A existing=()
    local -a obsolete=()
    local package_pattern="^sidenote-(${latest:2:-1})-latest-(full|delta)\.nupkg$"
    if [[ $rolling_state == exists ]]; then
        inventory=$(latest_inventory) || exit 1
        while IFS=$'\t' read -r name digest; do
            if [[ -n $name ]]; then existing[$name]=$digest; fi
        done <<< "$inventory"
        if [[ $rolling_draft == false ]]; then
            if [[ -n ${existing[releases.latest.json]+x} ]]; then
                published_version=$(published_latest_version) || exit 1
            else
                feed_missing=true
                published_version=$(retained_latest_version "$package_pattern" "${!existing[@]}") || exit 1
            fi
            if ! published_cmp=$(version_cmp "$tag" "v$published_version" 5 "$latest"); then
                echo 'Could not identify the published Latest package version.' >&2
                exit 1
            fi
            if [[ $published_cmp == -1 && ( $latest_status != identical || $feed_missing == true ) ]]; then
                echo "Package $tag is older than published Latest v$published_version; skipping Latest publish."
                return 0
            fi
            if [[ $feed_missing == true && $published_cmp == 0 ]]; then
                [[ -f $feed && -f "$asset_dir/sidenote-${tag#v}-latest-full.nupkg" ]] || {
                    echo 'The proved feed or its Latest full package is missing.' >&2
                    exit 1
                }
                repair_feed=true
            fi
        fi
    fi
    if [[ $latest_status == identical || $published_cmp == 0 ]]; then
        # The feed is uploaded last, so a same-version repair must find the other proved files intact.
        for asset in "${assets[@]}"; do
            if [[ $repair_feed == true && $asset == "$feed" ]]; then continue; fi
            if ! latest_asset_matches "$asset"; then
                echo "Latest asset ${asset##*/} is missing or differs from the proved package; refusing to resume." >&2
                exit 1
            fi
        done
        if [[ $repair_feed == true ]]; then
            run_write gh release upload latest-main "$feed" --repo "$repo"
        elif [[ $latest_status == identical && $rolling_draft == false && $rolling_name == "Sidenote latest $tag" ]]; then
            published_notes=$(read_or_fail 'Failed to read Latest release notes.' gh release view latest-main \
                --repo "$repo" --json isPrerelease,body --jq 'select(.isPrerelease == true) | .body') || exit 1
            if [[ $published_notes == "$(<"$notes_file")" ]]; then
                echo "Latest already publishes the proved package $tag; skipping Latest publish."
                return 0
            fi
        fi
        finish_latest_release
        return
    fi
    for asset in "$asset_dir/"*.nupkg; do
        name=${asset##*/}
        if [[ -n ${existing[$name]+x} ]]; then
            if ! latest_asset_matches "$asset"; then
                echo "Existing asset $name differs from the proved package; refusing to replace it." >&2
                exit 1
            fi
        else
            run_write gh release upload latest-main "$asset" --repo "$repo"
            existing[$name]=''
        fi
    done

    # The Latest version grammar has fixed labels, so version-sort orders its numeric components.
    while IFS=$'\t' read -r version name; do
        if [[ -z $version ]]; then continue; fi
        if [[ $version != "$previous" ]]; then
            count=$((count + 1))
            previous=$version
        fi
        if (( count > 3 )); then
            [[ v$version != "$tag" ]] || { echo 'The candidate is older than the retained Latest builds.' >&2; exit 1; }
            obsolete+=("$name")
        fi
    done < <(
        for name in "${!existing[@]}"; do
            if [[ $name =~ $package_pattern ]]; then
                printf '%s\t%s\n' "${BASH_REMATCH[1]}" "$name"
            fi
        done | sort -t $'\t' -k1,1Vr
    )

    run_write gh release upload latest-main "$asset_dir/sidenote-latest-Setup.exe" "$asset_dir/SHA256SUMS" --repo "$repo" --clobber
    run_write gh release upload latest-main "$feed" --repo "$repo" --clobber
    for name in "${!existing[@]}"; do
        if [[ $name == *.pending || $name == *.previous-* ]]; then obsolete+=("$name"); fi
    done
    for name in "${obsolete[@]}"; do
        run_write gh release delete-asset latest-main "$name" --repo "$repo" --yes
    done
    finish_latest_release
}

stable_latest=true
if [[ $channel == stable ]]; then
    stable_tags=$(read_or_fail 'Failed to list published stable releases.' gh api "repos/$repo/releases" --paginate --jq '.[] | select(.draft == false and .prerelease == false) | .tag_name') || exit 1
    while IFS= read -r published_tag; do
        [[ -n $published_tag ]] || continue
        if cmp=$(version_cmp "$published_tag" "$tag" 3 "$stable") && [[ $cmp == 1 ]]; then
            stable_latest=false
            break
        fi
    done <<< "$stable_tags"
    echo "Stable latest flag: $stable_latest"
fi

create=(gh release create "$tag" --repo "$repo" --verify-tag --title "$tag" --notes-file "$notes_file" --draft)
upload=(gh release upload "$tag" "${assets[@]}" --repo "$repo")
upload_clobber=(gh release upload "$tag" "${assets[@]}" --repo "$repo" --clobber)
publish_release=(gh release edit "$tag" --repo "$repo" --draft=false --notes-file "$notes_file")
if [[ $channel == stable ]]; then
    create+=("--latest=$stable_latest")
    publish_release+=("--latest=$stable_latest")
else
    create+=(--prerelease --latest=false)
    publish_release+=(--prerelease --latest=false)
fi

versioned_state=rolling-only
if [[ $channel != latest ]]; then
    versioned_lookup_status=0
    versioned_is_draft=$(lookup_or_missing 'release not found' "Failed to look up release $tag." gh release view "$tag" --repo "$repo" --json isDraft --jq .isDraft) || versioned_lookup_status=$?
    case $versioned_lookup_status in
        0)
            if [[ $versioned_is_draft == true ]]; then versioned_state=draft; else versioned_state=published; fi
            ;;
        1) versioned_state=missing ;;
        *) exit 1 ;;
    esac
    echo "Versioned release branch: $versioned_state"
fi

if [[ $channel == latest ]]; then
    # The publish job serializes Latest writes, so read the tag here after acquiring that slot.
    main_status=$(compare_status "$sha" heads/main) || exit 1
    case $main_status in
        ahead|identical) ;;
        behind|diverged)
            echo "Commit $sha is outside main history ($main_status); skipping Latest publish."
            exit 0
            ;;
        *) echo "Unexpected main comparison status: $main_status" >&2; exit 1 ;;
    esac
    latest_tag_exists=$(read_or_fail 'Failed to look up the latest-main tag.' \
        gh api "repos/$repo/git/matching-refs/tags/latest-main" |
        jq -r 'def valid_refs:
            if type == "array" then all(.[]; if type == "object" then (.ref | type) == "string" else false end)
            else false end;
            if valid_refs then any(.[]; .ref == "refs/tags/latest-main")
            else error("Malformed latest-main tag lookup response") end') || exit 1
    latest_status=missing
    case $latest_tag_exists in
        true)
            latest_status=$(compare_status tags/latest-main "$sha") || exit 1
            case $latest_status in
                ahead|identical) ;;
                behind|diverged)
                    echo "Commit $sha is $latest_status relative to latest-main; skipping Latest publish."
                    exit 0
                    ;;
                *) echo "Unexpected latest-main comparison status: $latest_status" >&2; exit 1 ;;
            esac
            ;;
        false) ;;
        *) echo "Unexpected latest-main tag lookup result: $latest_tag_exists" >&2; exit 1 ;;
    esac
elif [[ $DRY_RUN == 1 ]]; then
    echo 'Dry run: skipping remote tag identity check.'
else
    resolved_tag_sha=$(remote_tag_sha) || exit 1
    if [[ $resolved_tag_sha != "$sha" ]]; then
        echo "Remote tag $tag points at $resolved_tag_sha, not $sha; refusing to publish." >&2
        exit 1
    fi
    echo "Remote tag $tag points at $sha."
fi

case $versioned_state in
    missing)
        run_write "${create[@]}"
        run_write "${upload[@]}"
        run_write "${publish_release[@]}"
        ;;
    draft)
        existing_assets=$(read_or_fail "Failed to list assets on draft release $tag." gh release view "$tag" --repo "$repo" --json assets --jq '.assets[].name') || exit 1
        declare -A wanted_asset_names=()
        for asset in "${assets[@]}"; do wanted_asset_names[$(basename -- "$asset")]=1; done
        while IFS= read -r existing_asset; do
            if [[ -n $existing_asset && -z ${wanted_asset_names[$existing_asset]+x} ]]; then
                run_write gh release delete-asset "$tag" "$existing_asset" --repo "$repo" --yes
            fi
        done <<< "$existing_assets"
        run_write "${upload_clobber[@]}"
        run_write "${publish_release[@]}"
        ;;
    published)
        echo "Versioned release $tag is already published; leaving it untouched."
        ;;
esac

if [[ $channel == dev || $channel == latest ]]; then
    rolling_tag=dev
    if [[ $channel == latest ]]; then
        rolling_tag=latest-main
    fi
    rolling_prefix="Sidenote $channel "
    rolling_create=(gh release create "$rolling_tag" "${assets[@]}" --repo "$repo" --target "$sha" --title "$rolling_prefix$tag" --notes-file "$notes_file" --prerelease --latest=false)
    rolling_delete=(gh release delete "$rolling_tag" --repo "$repo" --cleanup-tag --yes)
    rolling_lookup_status=0
    rolling_info=$(lookup_or_missing 'release not found' "Failed to look up release $rolling_tag." gh release view "$rolling_tag" --repo "$repo" --json isDraft,name --jq '(.isDraft | tostring) + " " + .name') || rolling_lookup_status=$?
    read -r rolling_draft rolling_name <<<"${rolling_info:-}"
    case $rolling_lookup_status in
        0) rolling_state=exists ;;
        1) rolling_state=missing ;;
        *) exit 1 ;;
    esac
    echo "Rolling $rolling_tag release branch: $rolling_state"
    rolling_replace=1
    # Resume interrupted drafts without comparing release titles.
    if [[ $rolling_state == exists && $rolling_draft != false ]]; then
        echo "Rolling $rolling_tag release is a draft (isDraft=$rolling_draft); replacing it."
    elif [[ $channel == dev && $rolling_state == exists ]]; then
        rolling_version=
        if [[ $rolling_name == "$rolling_prefix"* ]]; then rolling_version=${rolling_name#$rolling_prefix}; fi
        if [[ -n $rolling_version ]] && cmp=$(version_cmp "$rolling_version" "$tag" 4 "$dev"); then
            if [[ $cmp != -1 ]]; then
                rolling_replace=0
                echo "Rolling $rolling_tag release is $rolling_version, the same as or newer than $tag; skipping rolling replacement."
            fi
        else
            echo "Rolling $rolling_tag release name '$rolling_name' has no readable $channel version; treating it as older."
        fi
    fi
    if [[ $channel == latest ]]; then
        if [[ $rolling_state == exists && $rolling_draft != true && $rolling_draft != false ]]; then
            echo "Unexpected Latest draft state: $rolling_draft" >&2
            exit 1
        fi
        if [[ $rolling_state == missing ]]; then
            if [[ $latest_status == identical ]]; then
                echo 'The latest-main tag matches the proved commit, but its release is missing.' >&2
                exit 1
            fi
            # An existing tag still guards against older runs while the draft is prepared.
            run_write gh release create latest-main --repo "$repo" --target "$sha" --title "$rolling_prefix$tag" \
                --notes-file "$notes_file" --draft --prerelease --latest=false
            rolling_draft=true
        fi
        refresh_latest_release
    elif (( rolling_replace )); then
        # The feed returns 404 between delete and create; the app treats that as a failed check, never a wrong package.
        if [[ $rolling_state == exists ]]; then
            run_write "${rolling_delete[@]}"
        else
            ref_lookup_status=0
            ref_info=$(lookup_or_missing 'not found|404' "Failed to look up git ref tags/$rolling_tag." gh api "repos/$repo/git/ref/tags/$rolling_tag") || ref_lookup_status=$?
            case $ref_lookup_status in
                0)
                    echo "Rolling $rolling_tag tag ref exists without a release; deleting it before create."
                    run_write gh api -X DELETE "repos/$repo/git/refs/tags/$rolling_tag"
                    ;;
                1) ;;
                *) exit 1 ;;
            esac
        fi
        run_write "${rolling_create[@]}"
    fi
fi
