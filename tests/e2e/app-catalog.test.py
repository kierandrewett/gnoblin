#!/usr/bin/env python3
"""Small offline checks for AppStream mapping and complete deterministic sharding."""

import gzip
import importlib.util
from pathlib import Path
import tempfile
import xml.etree.ElementTree as ET


path = Path(__file__).with_name("app-catalog.py")
spec = importlib.util.spec_from_file_location("gnoblin_app_catalog", path)
catalog_module = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(catalog_module)

fixture = ET.fromstring("""
<component type="desktop-application">
  <id>org.example.Editor</id><name>Editor</name><summary>Text</summary>
  <pkgname>example-editor</pkgname>
  <categories><category>Development</category><category>Utility</category></categories>
  <launchable type="desktop-id">org.example.Editor.desktop</launchable>
</component>
""")
parsed = catalog_module.parse_component(fixture)
assert parsed["app_id"] == "org.example.Editor"
assert parsed["install"] == "example-editor"
assert parsed["launch"] == "org.example.Editor"
assert parsed["category"] == "Development"
legacy = catalog_module.parse_component(
    ET.fromstring(
        """
<component type="desktop">
  <id>org.example.Legacy.desktop</id><name>Legacy</name><pkgname>legacy</pkgname>
</component>
"""
    )
)
assert legacy["app_id"] == "org.example.Legacy"
assert legacy["launch"] == "org.example.Legacy"

with tempfile.TemporaryDirectory() as directory:
    stream = Path(directory) / "fedora.xml.gz"
    xml = ET.Element("components")
    for index in range(17):
        item = ET.SubElement(xml, "component", {"type": "desktop-application"})
        ET.SubElement(item, "id").text = f"org.example.App{index}"
        ET.SubElement(item, "name").text = f"App {index}"
        ET.SubElement(item, "pkgname").text = f"example-app-{index}"
        categories = ET.SubElement(item, "categories")
        ET.SubElement(categories, "category").text = "Game" if index % 2 else "Utility"
        ET.SubElement(item, "launchable", {"type": "desktop-id"}).text = f"org.example.App{index}.desktop"
    with gzip.open(stream, "wb") as compressed:
        compressed.write(ET.tostring(xml))

    selected = catalog_module.fetch_fedora(stream, 15, {"org.example.App0"})
    assert len(selected) == 15
    assert all(app["app_id"] != "org.example.App0" for app in selected)
    full = {
        "schema": 1,
        "generated_utc": "now",
        "counts": {"flathub_popular": 12, "fedora_appstream": 15},
        "apps": ([{"app_id": f"flat-{index}", "source": "flathub-popular"} for index in range(12)] + selected),
    }
    shards = [catalog_module.shard_catalog(full, index, 5) for index in range(5)]
    identities = [app["app_id"] for shard in shards for app in shard["apps"]]
    assert len(identities) == len(set(identities)) == len(full["apps"])
    assert sum(len(shard["apps"]) for shard in shards) == len(full["apps"])

targeted = catalog_module.select_app_ids(shards[0], ["flat-10", "flat-0"])
assert targeted["shard"] == shards[0]["shard"]
assert [app["app_id"] for app in targeted["apps"]] == ["flat-10", "flat-0"]
try:
    catalog_module.select_app_ids(shards[0], ["missing-app"])
except ValueError as error:
    assert "missing-app" in str(error)
else:
    raise AssertionError("targeting an app outside the selected shard must fail")

print("PASS: AppStream mapping, deterministic sharding, and targeted app selection")
