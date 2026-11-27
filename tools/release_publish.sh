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
    local left=$1 right=$2 count=$3 regex=$4 i
    [[ $left =~ $regex ]] || return 1
    local -a left_parts=("${BASH_REMATCH[@]:1:$count}")
    [[ $right =~ $regex ]] || return 1
    local -a right_parts=("${BASH_REMATCH[@]:1:$count}")
    for ((i = 0; i < count; i++)); do
        if (( left_parts[i] < right_parts[i] )); then printf '%s\n' -1; return 0; fi
        if (( left_parts[i] > right_parts[i] )); then printf '%s\n' 1; return 0; fi
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

refresh_latest_release() {
    local staging asset name id inventory previous i
    local -a ordered=() pending=() promoted=()
    local -A ids=() wanted=()
    local asset_api="repos/$repo/releases/assets"
    staging=$(mktemp -d "$runner_temp/latest-assets.XXXXXXXX")
    # Keep the live feed pointing to its old packages until the new payload is ready.
    for asset in "$asset_dir/"*.nupkg "$asset_dir/sidenote-latest-Setup.exe" \
        "$asset_dir/SHA256SUMS" "$asset_dir/releases.latest.json"; do
        name=${asset##*/}
        ordered+=("$name")
        wanted[$name]=1
        cp -- "$asset" "$staging/$name.pending"
        pending+=("$staging/$name.pending")
    done
    run_write gh release upload latest-main "${pending[@]}" --repo "$repo" --clobber
    if [[ $DRY_RUN == 1 ]]; then return; fi
    inventory=$(read_or_fail 'Failed to list staged Latest assets.' gh release view latest-main --repo "$repo" \
        --json assets --jq '.assets[] | [.name, (.apiUrl | split("/") | last)] | @tsv') || exit 1
    while IFS=$'\t' read -r name id; do
        if [[ -n $name && $id =~ ^[0-9]+$ ]]; then ids[$name]=$id; fi
    done <<< "$inventory"
    for name in "${ordered[@]}"; do
        [[ -n ${ids[$name.pending]+x} ]] || { echo "Missing staged asset $name." >&2; exit 1; }
    done
    for name in "${ordered[@]}"; do
        previous=${ids[$name]:-}
        if [[ -n $previous ]] && ! run_write gh api -X PATCH "$asset_api/$previous" -f "name=$name.previous-$previous" --silent; then
            run_write gh api -X PATCH "$asset_api/$previous" -f "name=$name" --silent || exit 1
            break
        fi
        if ! run_write gh api -X PATCH "$asset_api/${ids[$name.pending]}" -f "name=$name" --silent; then
            if [[ -n $previous ]]; then
                # If the index rename took effect but returned an error, keep the packages it may now reference.
                run_write gh api -X PATCH "$asset_api/$previous" -f "name=$name" --silent || exit 1
            elif [[ $name == releases.latest.json ]]; then
                exit 1
            fi
            break
        fi
        promoted+=("$name")
    done
    if (( ${#promoted[@]} != ${#ordered[@]} )); then
        for ((i = ${#promoted[@]} - 1; i >= 0; i--)); do
            name=${promoted[i]}
            previous=${ids[$name]:-}
            if run_write gh api -X PATCH "$asset_api/${ids[$name.pending]}" -f "name=$name.pending" --silent && [[ -n $previous ]]; then
                run_write gh api -X PATCH "$asset_api/$previous" -f "name=$name" --silent || echo "Could not restore $name." >&2
            fi
        done
        exit 1
    fi
    if [[ $rolling_draft == false ]]; then
        run_write gh api -X PATCH "repos/$repo/git/refs/tags/latest-main" -f "sha=$sha" -F force=true --silent
    fi
    inventory=$(read_or_fail 'Failed to list current Latest assets.' gh release view latest-main --repo "$repo" --json assets --jq '.assets[].name') || exit 1
    while IFS= read -r name; do
        if [[ -n $name && -z ${wanted[$name]+x} ]]; then
            run_write gh release delete-asset latest-main "$name" --repo "$repo" --yes
        fi
    done <<< "$inventory"
    run_write gh release edit latest-main --repo "$repo" --target "$sha" --title "Sidenote latest $tag" \
        --notes-file "$notes_file" --draft=false --prerelease --latest=false
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
    main_sha=$(read_or_fail 'Failed to resolve main.' gh api "repos/$repo/git/ref/heads/main" --jq .object.sha) || exit 1
    if [[ $main_sha != "$sha" ]]; then
        echo 'Main advanced during packaging; skipping Latest publish.'
        exit 0
    fi
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
    rolling_regex=$dev
    rolling_parts=4
    if [[ $channel == latest ]]; then
        rolling_tag=latest-main
        rolling_regex=$latest
        rolling_parts=5
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
    # A draft is an interrupted create: replace it even at the same version, or the feed stays down.
    if [[ $rolling_state == exists && $rolling_draft != false ]]; then
        echo "Rolling $rolling_tag release is a draft (isDraft=$rolling_draft); replacing it."
    elif [[ $rolling_state == exists ]]; then
        rolling_version=
        if [[ $rolling_name == "$rolling_prefix"* ]]; then rolling_version=${rolling_name#$rolling_prefix}; fi
        if [[ -n $rolling_version ]] && cmp=$(version_cmp "$rolling_version" "$tag" "$rolling_parts" "$rolling_regex"); then
            if [[ $cmp != -1 ]]; then
                rolling_replace=0
                echo "Rolling $rolling_tag release is $rolling_version, the same as or newer than $tag; skipping rolling replacement."
            fi
        else
            echo "Rolling $rolling_tag release name '$rolling_name' has no readable $channel version; treating it as older."
        fi
    fi
    if (( rolling_replace )) && [[ $channel == latest && $rolling_state == exists ]]; then
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
