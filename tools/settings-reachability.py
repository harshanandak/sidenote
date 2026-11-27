"""Verify every dev.9 setting key against the rendered Settings element trees."""
import argparse
import json
from pathlib import Path


HIDDEN_PAGES = ("Connected apps", "Sync")
HIDDEN_CONTROLS = {"mcp-connect", "feature-Other devices", "sync-folder", "relay-preview", "relay-address"}


def nodes(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from nodes(child)
    elif isinstance(value, list):
        for child in value:
            yield from nodes(child)


def entered_query(tree, input_key, query):
    return any(node.get("path", "").endswith(f"/{input_key}/value/input") and node.get("text") == query
               for node in nodes(tree))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("folder", nargs="?", type=Path, default=Path("artifacts/settings"))
    parser.add_argument("--channel", choices=("stable", "dev"), default="dev")
    args = parser.parse_args()
    folder = args.folder
    stable = args.channel == "stable"
    manifest = json.loads((Path(__file__).parent / "ui-fixtures/settings-legacy-keys.json").read_text())
    trees = list(folder.glob("settings-*-1280-light.json"))
    assert trees, f"No render trees in {folder}"
    for tree in folder.glob("settings-*.json"):
        assert not json.loads(tree.read_text(encoding="utf-8-sig"))["warnings"], f"Render warnings in {tree}"
    all_nodes = [node for tree in trees for node in nodes(json.loads(tree.read_text(encoding="utf-8-sig")))]
    keys = {node["key"] for node in all_nodes if isinstance(node.get("key"), str)}
    if stable:
        hidden = HIDDEN_CONTROLS | {f"find-setting-{key}" for key in HIDDEN_CONTROLS}
        hidden |= {f"settings-page-{page}" for page in HIDDEN_PAGES} | {"privacy-connected-apps"}
        assert keys.isdisjoint(hidden), f"Stable Settings exposes {sorted(keys & hidden)}"
    missing = {key: [control for control in controls if control not in keys]
               for key, controls in manifest["keys"].items()
               if not stable or key != "features.other_devices"}
    missing = {key: controls for key, controls in missing.items() if controls}
    assert not missing, f"Unreachable legacy settings: {missing}"
    for name in ("General", "Meetings", "Prompt library", "Tasks", "Hex and dictation", "Models and agents", "Connected apps", "Appearance and startup", "Sync", "Privacy and data", "Updates", "Diagnostics"):
        assert (f"settings-page-{name}" in keys) == (not stable or name not in HIDDEN_PAGES), name
    assert "settings-page-People" not in keys, "People belongs to the home sidebar, not Settings"
    search = json.loads((folder / "settings-search-1280-light.json").read_text(encoding="utf-8-sig"))
    search_nodes = list(nodes(search))
    assert any(node.get("key") == "hs-meetings.silence_threshold" and node.get("rect", [0,-1,0,0])[1] >= 108 and node.get("rect", [0,0,0,0])[1] + node.get("rect", [0,0,0,0])[3] <= 800 and node.get("fill") == "#fff3c4ff" for node in search_nodes), "Search did not reveal the Advanced row"
    assert any(node.get("key") == "settings-advanced" and node.get("expanded") for node in search_nodes), "Search did not open Advanced"
    mapping=json.loads((Path(__file__).parent / "ui-fixtures/settings-search-map.json").read_text(encoding="utf-8"))
    assert len(mapping)==len(manifest["keys"])==50
    for item in mapping:
        assert item["key"] in manifest["keys"],item["key"]
        assert item["preserve_controls"]==manifest["keys"][item["key"]],item["key"]
    setup_states = {
        "hex-setup": ("Install Hex", "Sidenote needs Hex"),
        "hex-installing": ("Install Hex", "12 of 48 MB (25%)"),
        "hex-install-failed": ("Try again", "Could not download Hex"),
        "hex-installed": ("Open Hex", "Hex is installed"),
    }
    for theme in ("dark", "light"):
        for width in (1280, 390):
            for state, (label, message) in setup_states.items():
                path = folder / f"settings-{state}-{width}-{theme}.json"
                tree = json.loads(path.read_text(encoding="utf-8-sig"))
                controls = {n["key"]: n for n in tree["elements"] if n.get("key")}
                assert "hex-setup" in controls, path
                progress = controls["hex-install-progress"]
                assert message in progress["text"], path
                assert progress["role"] == ("alert" if state == "hex-install-failed" else "status"), path
                action = "hex-setup-open" if state == "hex-installed" else "hex-install"
                assert controls[action]["label"] == label, path
                recovery = ("hex-download-page", "hex-install-recheck")
                actions = (action,)
                if state == "hex-installed":
                    assert all(key not in controls for key in recovery), path
                else:
                    actions += recovery
                for key in actions:
                    control = controls[key]
                    assert control["focusable"] == (state != "hex-installing" or key == "hex-download-page"), (path, key)
                    x, y, w, h = control["rect"]
                    assert 0 <= x < x + w <= width and 0 <= y < y + h <= 900, (path, key)
            for state in ("hex-meetings-off", "hex-meetings-enable"):
                path = folder / f"settings-{state}-{width}-{theme}.json"
                tree = json.loads(path.read_text(encoding="utf-8-sig"))
                controls = {n["key"]: n for n in tree["elements"] if n.get("key")}
                assert "hex-meetings-setup" in controls, path
                assert "Settings > Labs > Meetings" in controls["hex-meetings-instructions"]["text"], path
                assert any(n.get("text") == "Turn on Meetings in Hex" for n in tree["elements"]), path
                assert any(n.get("text") == "Hex is running. Check its status here before recording." for n in tree["elements"]), path
                assert ("hex-meetings-enable" in controls) == (state == "hex-meetings-enable"), path
                actions = ["hex-meetings-recheck"] + (["hex-meetings-enable"] if state == "hex-meetings-enable" else [])
                for key in actions:
                    assert controls[key]["focusable"], (path, key)
                    x, y, w, h = controls[key]["rect"]
                    assert 0 <= x < x + w <= width and 0 <= y < y + h <= 900, (path, key)
    if not stable:
        connected = json.loads((folder / "settings-connected-apps-1280-light.json").read_text(encoding="utf-8-sig"))
        assert any(n.get("key") == "mcp-connect" for n in nodes(connected))
    privacy = json.loads((folder / "settings-privacy-data-1280-light.json").read_text(encoding="utf-8-sig"))
    assert any(n.get("key") == "privacy-connected-apps" for n in nodes(privacy)) == (not stable)
    assert not any(n.get("key") == "mcp-connect" for n in nodes(privacy))
    for query in ("MCP", "connect", "Claude", "Codex", "Cursor", "OpenCode"):
        tree = json.loads((folder / f"settings-mcp-search-{query}-1280-light.json").read_text(encoding="utf-8-sig"))
        assert entered_query(tree, "hex-filter", query), query
        assert any(n.get("key") == "find-setting-mcp-connect" for n in nodes(tree)) == (not stable), query
    if stable:
        hidden_titles = {f"Settings · {page}" for page in HIDDEN_PAGES}
        for query in HIDDEN_PAGES:
            search_tree = json.loads((folder / f"settings-hidden-search-{query}-1280-light.json").read_text(encoding="utf-8-sig"))
            assert entered_query(search_tree, "hex-filter", query), query
            finder = json.loads((folder / f"settings-hidden-finder-{query}-1280-light.json").read_text(encoding="utf-8-sig"))
            assert entered_query(finder, "finder-input", query), query
            labels = {n.get("label") for n in nodes(finder) if (n.get("key") or "").startswith("finder-row-")}
            assert labels.isdisjoint(hidden_titles), (query, labels & hidden_titles)
        print("PASS: Stable excludes Sync and Connected apps from Settings, Privacy and Ctrl+K.")
    print(f"PASS: {args.channel} legacy setting keys, sidebar and search reveal verified in {len(trees)} render trees.")
    print("PASS: all 16 Hex setup renders show the expected copy, controls, busy state and visible actions.")
    print("PASS: all 8 Meetings-off renders show instructions, re-check and only supported enable actions.")


if __name__ == "__main__":
    main()
