"""Reject a tag that does not match all shipped application versions."""
import pathlib, plistlib, re, sys, tomllib
root = pathlib.Path(__file__).resolve().parent.parent
tag = sys.argv[1]
assert re.fullmatch(r"v\d+\.\d+\.\d+", tag), "Use vMAJOR.MINOR.PATCH"
version = tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"]
assert tag == "v" + version, "Tag and Cargo version differ"
plist = plistlib.loads((root / "packaging/macos/Info.plist").read_bytes())
assert plist["CFBundleShortVersionString"] == version
assert plist["CFBundleVersion"] == version
print("Release versions match:", tag)
