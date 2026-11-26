#!/usr/bin/env bash
set -euo pipefail

if [[ ${DRY_RUN:-} != 0 && ${DRY_RUN:-} != 1 ]]; then
    echo 'DRY_RUN must be exactly 0 or 1.' >&2
    exit 1
fi
if [[ $DRY_RUN == 0 && ${GITHUB_EVENT_NAME:-} != push ]]; then
    echo 'DRY_RUN=0 is allowed only for a tag push.' >&2
    exit 1
fi

tag=${RELEASE_TAG:?Set RELEASE_TAG}
repo=${GITHUB_REPOSITORY:?Set GITHUB_REPOSITORY}
asset_dir=${ASSET_DIR:?Set ASSET_DIR}
stable='^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$'
dev='^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-dev\.(0|[1-9][0-9]*)$'
if [[ $tag =~ $stable ]]; then
    channel=stable
elif [[ $tag =~ $dev ]]; then
    channel=dev
else
    echo "Unexpected release tag: $tag" >&2
    exit 1
fi

notes_source="docs/releases/$tag.md"
notes_file="$RUNNER_TEMP/sidenote-$tag-notes.md"
if [[ -f $notes_source ]]; then
    cat "$notes_source" > "$notes_file"
elif [[ $GITHUB_EVENT_NAME == push ]]; then
    echo "Missing required release notes: $notes_source" >&2
    exit 1
else
    echo "$notes_source is missing; using placeholder notes for this dry run."
    printf 'Placeholder release notes for %s.\n' "$tag" > "$notes_file"
fi
cat >>"$notes_file" <<'EOF'

## Verify your download

Download `SHA256SUMS` from the same release, then run `Get-FileHash <file> -Algorithm SHA256` in PowerShell and compare the result with the matching line in `SHA256SUMS`.
EOF
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
    if [[ $DRY_RUN == 0 ]]; then
        "$@"
    else
        print_command "$@"
    fi
}

create=(gh release create "$tag" --repo "$repo" --verify-tag --title "$tag" --notes-file "$notes_file" --draft)
upload=(gh release upload "$tag" "${assets[@]}" --repo "$repo")
upload_clobber=(gh release upload "$tag" "${assets[@]}" --repo "$repo" --clobber)
publish_release=(gh release edit "$tag" --repo "$repo" --draft=false)
if [[ $channel == stable ]]; then
    create+=(--latest=true)
    publish_release+=(--latest=true)
else
    create+=(--prerelease --latest=false)
    publish_release+=(--prerelease --latest=false)
fi

versioned_state=missing
if is_draft=$(gh release view "$tag" --repo "$repo" --json isDraft --jq .isDraft 2>/dev/null); then
    if [[ $is_draft == true ]]; then
        versioned_state=draft
    else
        versioned_state=published
    fi
fi
echo "Versioned release branch: $versioned_state"
case $versioned_state in
    missing)
        run_write "${create[@]}"
        run_write "${upload[@]}"
        run_write "${publish_release[@]}"
        ;;
    draft)
        run_write "${upload_clobber[@]}"
        run_write "${publish_release[@]}"
        ;;
    published)
        echo "Versioned release $tag is already published; leaving it untouched."
        ;;
esac

if [[ $channel == dev ]]; then
    rolling_create=(gh release create dev "${assets[@]}" --repo "$repo" --target "$GITHUB_SHA" --title 'Sidenote dev' --notes-file "$notes_file" --prerelease --latest=false)
    rolling_delete=(gh release delete dev --repo "$repo" --cleanup-tag --yes)
    rolling_state=missing
    if gh release view dev --repo "$repo" --json tagName >/dev/null 2>&1; then
        rolling_state=exists
    fi
    echo "Rolling dev release branch: $rolling_state"
    # The feed returns 404 between delete and create; the app treats that as a failed check, never a wrong package.
    if [[ $rolling_state == exists ]]; then
        run_write "${rolling_delete[@]}"
    fi
    run_write "${rolling_create[@]}"
fi
